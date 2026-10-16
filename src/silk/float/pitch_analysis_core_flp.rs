//! Mirror of `silk/float/pitch_analysis_core_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The 4 kHz + 8 kHz + full-rate (stage 3) pitch estimator.
//!
//! `celt_pitch_xcorr` is the arch-4 AVX2 kernel
//! (`celt::kernels::celt_pitch_xcorr`), which is what the reference build
//! executes; the C `arch` argument is therefore dropped.
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
use crate::celt::kernels::celt_pitch_xcorr;
use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::inner_product_flp::silk_inner_product_FLP;
use crate::silk::float::sigproc_flp::{
    silk_float, silk_float2short_array, silk_log2, silk_short2float_array,
};
use crate::silk::float::sort_flp::silk_insertion_sort_decreasing_FLP;
use crate::silk::macros::*;
use crate::silk::pitch_est_tables::*;
use crate::silk::resampler_down2::silk_resampler_down2;
use crate::silk::resampler_down2_3::silk_resampler_down2_3;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

const SCRATCH_SIZE: usize = 22;

/* `silk/pitch_est_defines.h` */
const PE_MAX_FS_KHZ: opus_int = 16;
const PE_MAX_NB_SUBFR: opus_int = 4;
const PE_SUBFR_LENGTH_MS: opus_int = 5;
const PE_LTP_MEM_LENGTH_MS: opus_int = 4 * PE_SUBFR_LENGTH_MS;
const PE_MAX_FRAME_LENGTH_MS: opus_int =
    PE_LTP_MEM_LENGTH_MS + PE_MAX_NB_SUBFR * PE_SUBFR_LENGTH_MS;
const PE_MAX_LAG_MS: opus_int = 18;
const PE_MIN_LAG_MS: opus_int = 2;
const PE_D_SRCH_LENGTH: opus_int = 24;
const PE_NB_STAGE3_LAGS: opus_int = 5;
const PE_NB_CBKS_STAGE2: opus_int = 3;
const PE_NB_CBKS_STAGE2_EXT: opus_int = 11;
const PE_NB_CBKS_STAGE3_MAX: opus_int = 34;
const PE_NB_CBKS_STAGE3_10MS: opus_int = 12;
const PE_NB_CBKS_STAGE2_10MS: opus_int = 3;
const PE_SHORTLAG_BIAS: silk_float = 0.2;
const PE_PREVLAG_BIAS: silk_float = 0.2;
const PE_FLATCONTOUR_BIAS: silk_float = 0.05;
const SILK_PE_MIN_COMPLEX: opus_int = 0;
const SILK_PE_MAX_COMPLEX: opus_int = 2;
const _PE_MAX_LAG: opus_int = PE_MAX_LAG_MS * PE_MAX_FS_KHZ;

/// Which lag codebook `Lag_CB_ptr` currently points at (the C reuses the
/// pointer for the stage-2 and stage-3 tables).
#[derive(Clone, Copy, PartialEq)]
enum LagCb {
    Stage2,
    Stage2_10ms,
    Stage3,
    Stage3_10ms,
}

/// `matrix_ptr(Lag_CB_ptr, i, j, cbk_size)` for the three lag codebooks.
#[inline(always)]
fn lag_cb_val(cb: LagCb, i: opus_int, j: opus_int) -> opus_int {
    match cb {
        LagCb::Stage2 => silk_CB_lags_stage2[i as usize][j as usize] as opus_int,
        LagCb::Stage2_10ms => silk_CB_lags_stage2_10_ms[i as usize][j as usize] as opus_int,
        LagCb::Stage3 => silk_CB_lags_stage3[i as usize][j as usize] as opus_int,
        LagCb::Stage3_10ms => silk_CB_lags_stage3_10_ms[i as usize][j as usize] as opus_int,
    }
}

/// `matrix_ptr(Lag_range_ptr, k, c, 2)` for the two stage-3 lag-range tables.
#[inline(always)]
fn lag_range_val(ten_ms: bool, complexity: opus_int, k: opus_int, c: opus_int) -> opus_int {
    if ten_ms {
        silk_Lag_range_stage3_10_ms[k as usize][c as usize] as opus_int
    } else {
        silk_Lag_range_stage3[complexity as usize][k as usize][c as usize] as opus_int
    }
}

/// The C `silk_ADD_SAT16(a, b)` macro applied to `silk_float` arguments:
/// `(opus_int16)silk_SAT16( (opus_int32)(a) + (b) )`.  With float arguments the
/// `(opus_int32)` cast truncates `a` towards zero, the addition is a `float`
/// addition (int32 converted to float), the saturation limits are the float
/// values `32767.0f` / `-32768.0f` and the final `opus_int16` cast truncates
/// the float towards zero again.
#[inline(always)]
fn add_sat16_float_args(a: silk_float, b: silk_float) -> silk_float {
    let sum = (a as opus_int32) as silk_float + b;
    let sat = if sum > silk_int16_MAX as silk_float {
        silk_int16_MAX as silk_float
    } else if sum < silk_int16_MIN as silk_float {
        silk_int16_MIN as silk_float
    } else {
        sum
    };
    (sat as opus_int16) as silk_float
}

/// `d_comp[i] += <int expr>` on an `opus_int16` element (int promotion, then
/// truncating assignment).
#[inline(always)]
fn dcomp_add(a: opus_int16, b: opus_int32) -> opus_int16 {
    (a as opus_int32).wrapping_add(b) as opus_int16
}

/************************************************************/
/* Internally used functions                                */
/************************************************************/
fn silk_P_Ana_calc_corr_st3(
    cross_corr_st3: &mut [[[silk_float; PE_NB_STAGE3_LAGS as usize]; PE_NB_CBKS_STAGE3_MAX as usize];
             PE_MAX_NB_SUBFR as usize],
    frame: &[silk_float],
    start_lag: opus_int,
    sf_length: opus_int,
    nb_subfr: opus_int,
    complexity: opus_int,
) {
    let mut i: opus_int;
    let mut j: opus_int;
    let mut k: opus_int;
    let mut lag_counter: opus_int;
    let mut lag_low: opus_int;
    let mut lag_high: opus_int;
    let mut nb_cbk_search: opus_int;
    let mut delta: opus_int;
    let mut idx: opus_int;
    let mut cbk_size: opus_int;
    let mut scratch_mem = [0.0f32; SCRATCH_SIZE];
    let mut xcorr = [0.0f32; SCRATCH_SIZE];
    let ten_ms: bool;
    let lag_cb: LagCb;

    celt_assert!(complexity >= SILK_PE_MIN_COMPLEX);
    celt_assert!(complexity <= SILK_PE_MAX_COMPLEX);

    if nb_subfr == PE_MAX_NB_SUBFR {
        ten_ms = false;
        lag_cb = LagCb::Stage3;
        nb_cbk_search = silk_nb_cbk_searchs_stage3[complexity as usize] as opus_int;
        cbk_size = PE_NB_CBKS_STAGE3_MAX;
    } else {
        celt_assert!(nb_subfr == PE_MAX_NB_SUBFR >> 1);
        ten_ms = true;
        lag_cb = LagCb::Stage3_10ms;
        nb_cbk_search = PE_NB_CBKS_STAGE3_10MS;
        cbk_size = PE_NB_CBKS_STAGE3_10MS;
    }
    let _ = cbk_size;

    /* target_ptr = &frame[ silk_LSHIFT( sf_length, 2 ) ] */
    let mut target_idx = silk_LSHIFT(sf_length, 2) as usize; /* Pointer to middle of frame */
    k = 0;
    while k < nb_subfr {
        lag_counter = 0;

        /* Calculate the correlations for each subframe */
        lag_low = lag_range_val(ten_ms, complexity, k, 0);
        lag_high = lag_range_val(ten_ms, complexity, k, 1);
        silk_assert!(lag_high - lag_low + 1 <= SCRATCH_SIZE as opus_int);
        celt_pitch_xcorr(
            &frame[target_idx..],
            &frame[target_idx - (start_lag + lag_high) as usize..],
            &mut xcorr,
            sf_length,
            lag_high - lag_low + 1,
        );
        j = lag_low;
        while j <= lag_high {
            silk_assert!(lag_counter < SCRATCH_SIZE as opus_int);
            scratch_mem[lag_counter as usize] = xcorr[(lag_high - j) as usize];
            lag_counter += 1;
            j += 1;
        }

        delta = lag_range_val(ten_ms, complexity, k, 0);
        i = 0;
        while i < nb_cbk_search {
            /* Fill out the 3 dim array that stores the correlations for */
            /* each code_book vector for each start lag */
            idx = lag_cb_val(lag_cb, k, i) - delta;
            j = 0;
            while j < PE_NB_STAGE3_LAGS {
                silk_assert!(idx + j < SCRATCH_SIZE as opus_int);
                silk_assert!(idx + j < lag_counter);
                cross_corr_st3[k as usize][i as usize][j as usize] =
                    scratch_mem[(idx + j) as usize];
                j += 1;
            }
            i += 1;
        }
        target_idx += sf_length as usize;
        k += 1;
    }
}

/********************************************************************/
/* Calculate the energies for first two subframes. The energies are */
/* calculated recursively.                                          */
/********************************************************************/
fn silk_P_Ana_calc_energy_st3(
    energies_st3: &mut [[[silk_float; PE_NB_STAGE3_LAGS as usize]; PE_NB_CBKS_STAGE3_MAX as usize];
             PE_MAX_NB_SUBFR as usize],
    frame: &[silk_float],
    start_lag: opus_int,
    sf_length: opus_int,
    nb_subfr: opus_int,
    complexity: opus_int,
) {
    let mut energy: f64;
    let mut k: opus_int;
    let mut i: opus_int;
    let mut j: opus_int;
    let mut lag_counter: opus_int;
    let mut nb_cbk_search: opus_int;
    let mut delta: opus_int;
    let mut idx: opus_int;
    let mut cbk_size: opus_int;
    let mut lag_diff: opus_int;
    let mut scratch_mem = [0.0f32; SCRATCH_SIZE];
    let ten_ms: bool;
    let lag_cb: LagCb;

    celt_assert!(complexity >= SILK_PE_MIN_COMPLEX);
    celt_assert!(complexity <= SILK_PE_MAX_COMPLEX);

    if nb_subfr == PE_MAX_NB_SUBFR {
        ten_ms = false;
        lag_cb = LagCb::Stage3;
        nb_cbk_search = silk_nb_cbk_searchs_stage3[complexity as usize] as opus_int;
        cbk_size = PE_NB_CBKS_STAGE3_MAX;
    } else {
        celt_assert!(nb_subfr == PE_MAX_NB_SUBFR >> 1);
        ten_ms = true;
        lag_cb = LagCb::Stage3_10ms;
        nb_cbk_search = PE_NB_CBKS_STAGE3_10MS;
        cbk_size = PE_NB_CBKS_STAGE3_10MS;
    }
    let _ = cbk_size;

    let mut target_idx = silk_LSHIFT(sf_length, 2) as usize;
    k = 0;
    while k < nb_subfr {
        lag_counter = 0;

        /* Calculate the energy for first lag */
        let basis_idx =
            target_idx - (start_lag + lag_range_val(ten_ms, complexity, k, 0)) as usize;
        energy = silk_energy_FLP(&frame[basis_idx..], sf_length) + 1e-3;
        silk_assert!(energy >= 0.0);
        scratch_mem[lag_counter as usize] = energy as silk_float;
        lag_counter += 1;

        lag_diff = lag_range_val(ten_ms, complexity, k, 1) - lag_range_val(ten_ms, complexity, k, 0)
            + 1;
        i = 1;
        while i < lag_diff {
            /* remove part outside new window */
            energy -= frame[basis_idx + (sf_length - i) as usize] as f64
                * frame[basis_idx + (sf_length - i) as usize] as f64;
            silk_assert!(energy >= 0.0);

            /* add part that comes into window */
            energy += frame[basis_idx - i as usize] as f64 * frame[basis_idx - i as usize] as f64;
            silk_assert!(energy >= 0.0);
            silk_assert!(lag_counter < SCRATCH_SIZE as opus_int);
            scratch_mem[lag_counter as usize] = energy as silk_float;
            lag_counter += 1;
            i += 1;
        }

        delta = lag_range_val(ten_ms, complexity, k, 0);
        i = 0;
        while i < nb_cbk_search {
            /* Fill out the 3 dim array that stores the correlations for    */
            /* each code_book vector for each start lag                     */
            idx = lag_cb_val(lag_cb, k, i) - delta;
            j = 0;
            while j < PE_NB_STAGE3_LAGS {
                silk_assert!(idx + j < SCRATCH_SIZE as opus_int);
                silk_assert!(idx + j < lag_counter);
                energies_st3[k as usize][i as usize][j as usize] = scratch_mem[(idx + j) as usize];
                silk_assert!(energies_st3[k as usize][i as usize][j as usize] >= 0.0f32);
                j += 1;
            }
            i += 1;
        }
        target_idx += sf_length as usize;
        k += 1;
    }
}

/************************************************************/
/* CORE PITCH ANALYSIS FUNCTION                             */
/************************************************************/
pub fn silk_pitch_analysis_core_FLP(
    frame: &[silk_float],
    pitch_out: &mut [opus_int],
    lagIndex: &mut opus_int16,
    contourIndex: &mut opus_int8,
    LTPCorr: &mut silk_float,
    prevLag: opus_int,
    search_thres1: silk_float,
    search_thres2: silk_float,
    Fs_kHz: opus_int,
    complexity: opus_int,
    nb_subfr: opus_int,
    arch: opus_int,
) -> opus_int {
    let mut i: opus_int;
    let mut k: opus_int;
    let mut d: opus_int;
    let mut j: opus_int;
    let mut frame_8kHz = [0.0f32; (PE_MAX_FRAME_LENGTH_MS * 8) as usize];
    let mut frame_4kHz = [0.0f32; (PE_MAX_FRAME_LENGTH_MS * 4) as usize];
    let mut frame_8_FIX = [0i16; (PE_MAX_FRAME_LENGTH_MS * 8) as usize];
    let mut frame_4_FIX = [0i16; (PE_MAX_FRAME_LENGTH_MS * 4) as usize];
    let mut filt_state = [0i32; 6];
    let mut threshold: silk_float;
    let mut contour_bias: silk_float;
    let mut C = [[0.0f32; ((_PE_MAX_LAG >> 1) + 5) as usize]; PE_MAX_NB_SUBFR as usize];
    let mut xcorr = [0.0f32; (PE_MAX_LAG_MS * 4 - PE_MIN_LAG_MS * 4 + 1) as usize];
    let mut CC = [0.0f32; PE_NB_CBKS_STAGE2_EXT as usize];
    let mut cross_corr: f64;
    let mut normalizer: f64;
    let mut energy: f64;
    let mut energy_tmp: f64;
    let mut d_srch = [0i32; PE_D_SRCH_LENGTH as usize];
    let mut d_comp = [0i16; ((_PE_MAX_LAG >> 1) + 5) as usize];
    let mut length_d_srch: opus_int;
    let mut length_d_comp: opus_int;
    let mut Cmax: silk_float;
    let mut CCmax: silk_float;
    let mut CCmax_b: silk_float;
    let mut CCmax_new_b: silk_float;
    let mut CCmax_new: silk_float;
    let mut CBimax: opus_int;
    let mut CBimax_new: opus_int;
    let mut lag: opus_int;
    let mut start_lag: opus_int;
    let mut end_lag: opus_int;
    let mut lag_new: opus_int;
    let mut cbk_size: opus_int;
    let mut lag_log2: silk_float;
    let mut prevLag_log2: silk_float;
    let mut delta_lag_log2_sqr: silk_float;
    let mut energies_st3 =
        [[[0.0f32; PE_NB_STAGE3_LAGS as usize]; PE_NB_CBKS_STAGE3_MAX as usize];
            PE_MAX_NB_SUBFR as usize];
    let mut cross_corr_st3 =
        [[[0.0f32; PE_NB_STAGE3_LAGS as usize]; PE_NB_CBKS_STAGE3_MAX as usize];
            PE_MAX_NB_SUBFR as usize];
    let mut lag_counter: opus_int;
    let frame_length: opus_int;
    let frame_length_8kHz: opus_int;
    let frame_length_4kHz: opus_int;
    let sf_length: opus_int;
    let sf_length_8kHz: opus_int;
    let sf_length_4kHz: opus_int;
    let min_lag: opus_int;
    let min_lag_8kHz: opus_int;
    let min_lag_4kHz: opus_int;
    let max_lag: opus_int;
    let max_lag_8kHz: opus_int;
    let max_lag_4kHz: opus_int;
    let mut nb_cbk_search: opus_int;
    let mut lag_cb: LagCb;
    let mut prevLag = prevLag;

    /* Check for valid sampling frequency */
    celt_assert!(Fs_kHz == 8 || Fs_kHz == 12 || Fs_kHz == 16);

    /* Check for valid complexity setting */
    celt_assert!(complexity >= SILK_PE_MIN_COMPLEX);
    celt_assert!(complexity <= SILK_PE_MAX_COMPLEX);

    silk_assert!(search_thres1 >= 0.0f32 && search_thres1 <= 1.0f32);
    silk_assert!(search_thres2 >= 0.0f32 && search_thres2 <= 1.0f32);

    /* Set up frame lengths max / min lag for the sampling frequency */
    frame_length = (PE_LTP_MEM_LENGTH_MS + nb_subfr * PE_SUBFR_LENGTH_MS) * Fs_kHz;
    frame_length_4kHz = (PE_LTP_MEM_LENGTH_MS + nb_subfr * PE_SUBFR_LENGTH_MS) * 4;
    frame_length_8kHz = (PE_LTP_MEM_LENGTH_MS + nb_subfr * PE_SUBFR_LENGTH_MS) * 8;
    sf_length = PE_SUBFR_LENGTH_MS * Fs_kHz;
    sf_length_4kHz = PE_SUBFR_LENGTH_MS * 4;
    sf_length_8kHz = PE_SUBFR_LENGTH_MS * 8;
    min_lag = PE_MIN_LAG_MS * Fs_kHz;
    min_lag_4kHz = PE_MIN_LAG_MS * 4;
    min_lag_8kHz = PE_MIN_LAG_MS * 8;
    max_lag = PE_MAX_LAG_MS * Fs_kHz - 1;
    max_lag_4kHz = PE_MAX_LAG_MS * 4;
    max_lag_8kHz = PE_MAX_LAG_MS * 8 - 1;

    /* Resample from input sampled at Fs_kHz to 8 kHz */
    if Fs_kHz == 16 {
        /* Resample to 16 -> 8 khz */
        let mut frame_16_FIX = [0i16; (16 * PE_MAX_FRAME_LENGTH_MS) as usize];
        silk_float2short_array(&mut frame_16_FIX, frame, frame_length);
        for v in filt_state.iter_mut().take(2) {
            *v = 0;
        }
        silk_resampler_down2(&mut filt_state, &mut frame_8_FIX, &frame_16_FIX, frame_length);
        silk_short2float_array(&mut frame_8kHz, &frame_8_FIX, frame_length_8kHz);
    } else if Fs_kHz == 12 {
        /* Resample to 12 -> 8 khz */
        let mut frame_12_FIX = [0i16; (12 * PE_MAX_FRAME_LENGTH_MS) as usize];
        silk_float2short_array(&mut frame_12_FIX, frame, frame_length);
        for v in filt_state.iter_mut().take(6) {
            *v = 0;
        }
        silk_resampler_down2_3(&mut filt_state, &mut frame_8_FIX, &frame_12_FIX, frame_length);
        silk_short2float_array(&mut frame_8kHz, &frame_8_FIX, frame_length_8kHz);
    } else {
        celt_assert!(Fs_kHz == 8);
        silk_float2short_array(&mut frame_8_FIX, frame, frame_length_8kHz);
    }

    /* Decimate again to 4 kHz */
    for v in filt_state.iter_mut().take(2) {
        *v = 0;
    }
    silk_resampler_down2(&mut filt_state, &mut frame_4_FIX, &frame_8_FIX, frame_length_8kHz);
    silk_short2float_array(&mut frame_4kHz, &frame_4_FIX, frame_length_4kHz);

    /* Low-pass filter */
    i = frame_length_4kHz - 1;
    while i > 0 {
        frame_4kHz[i as usize] =
            add_sat16_float_args(frame_4kHz[i as usize], frame_4kHz[(i - 1) as usize]);
        i -= 1;
    }

    /******************************************************************************
    * FIRST STAGE, operating in 4 khz
    ******************************************************************************/
    for row in C.iter_mut().take(nb_subfr as usize) {
        for v in row.iter_mut() {
            *v = 0.0;
        }
    }
    let mut target_idx = silk_LSHIFT(sf_length_4kHz, 2) as usize;
    k = 0;
    while k < nb_subfr >> 1 {
        /* Check that we are within range of the array */
        /* target_idx is a usize index into frame_4kHz, so it is >= 0 by construction */
        celt_assert!(target_idx + sf_length_8kHz as usize <= frame_length_4kHz as usize);

        let mut basis_idx = target_idx - min_lag_4kHz as usize;

        /* Check that we are within range of the array */
        celt_assert!(basis_idx + sf_length_8kHz as usize <= frame_length_4kHz as usize);

        celt_pitch_xcorr(
            &frame_4kHz[target_idx..],
            &frame_4kHz[target_idx - max_lag_4kHz as usize..],
            &mut xcorr,
            sf_length_8kHz,
            max_lag_4kHz - min_lag_4kHz + 1,
        );

        /* Calculate first vector products before loop */
        cross_corr = xcorr[(max_lag_4kHz - min_lag_4kHz) as usize] as f64;
        normalizer = silk_energy_FLP(&frame_4kHz[target_idx..], sf_length_8kHz)
            + silk_energy_FLP(&frame_4kHz[basis_idx..], sf_length_8kHz)
            + sf_length_8kHz as f64 * 4000.0f32 as f64;

        C[0][min_lag_4kHz as usize] += (2.0f64 * cross_corr / normalizer) as silk_float;

        /* From now on normalizer is computed recursively */
        d = min_lag_4kHz + 1;
        while d <= max_lag_4kHz {
            basis_idx -= 1;

            /* Check that we are within range of the array */
            silk_assert!(basis_idx as isize >= 0);
            silk_assert!(basis_idx + sf_length_8kHz as usize <= frame_length_4kHz as usize);

            cross_corr = xcorr[(max_lag_4kHz - d) as usize] as f64;

            /* Add contribution of new sample and remove contribution from oldest sample */
            normalizer += frame_4kHz[basis_idx] as f64 * frame_4kHz[basis_idx] as f64
                - frame_4kHz[basis_idx + sf_length_8kHz as usize] as f64
                    * frame_4kHz[basis_idx + sf_length_8kHz as usize] as f64;
            C[0][d as usize] += (2.0f64 * cross_corr / normalizer) as silk_float;
            d += 1;
        }
        /* Update target pointer */
        target_idx += sf_length_8kHz as usize;
        k += 1;
    }

    /* Apply short-lag bias */
    i = max_lag_4kHz;
    while i >= min_lag_4kHz {
        C[0][i as usize] -= C[0][i as usize] * i as silk_float / 4096.0f32;
        i -= 1;
    }

    /* Sort */
    length_d_srch = 4 + 2 * complexity;
    celt_assert!(3 * length_d_srch <= PE_D_SRCH_LENGTH);
    silk_insertion_sort_decreasing_FLP(
        &mut C[0][min_lag_4kHz as usize..],
        &mut d_srch,
        max_lag_4kHz - min_lag_4kHz + 1,
        length_d_srch,
    );

    /* Escape if correlation is very low already here */
    Cmax = C[0][min_lag_4kHz as usize];
    if Cmax < 0.2f32 {
        for v in pitch_out.iter_mut().take(nb_subfr as usize) {
            *v = 0;
        }
        *LTPCorr = 0.0;
        *lagIndex = 0;
        *contourIndex = 0;
        return 1;
    }

    threshold = search_thres1 * Cmax;
    i = 0;
    while i < length_d_srch {
        /* Convert to 8 kHz indices for the sorted correlation that exceeds the threshold */
        if C[0][(min_lag_4kHz + i) as usize] > threshold {
            d_srch[i as usize] = silk_LSHIFT(d_srch[i as usize] + min_lag_4kHz, 1);
        } else {
            length_d_srch = i;
            break;
        }
        i += 1;
    }
    celt_assert!(length_d_srch > 0);

    i = min_lag_8kHz - 5;
    while i < max_lag_8kHz + 5 {
        d_comp[i as usize] = 0;
        i += 1;
    }
    i = 0;
    while i < length_d_srch {
        d_comp[d_srch[i as usize] as usize] = 1;
        i += 1;
    }

    /* Convolution */
    i = max_lag_8kHz + 3;
    while i >= min_lag_8kHz {
        d_comp[i as usize] = dcomp_add(
            d_comp[i as usize],
            d_comp[(i - 1) as usize] as opus_int32 + d_comp[(i - 2) as usize] as opus_int32,
        );
        i -= 1;
    }

    length_d_srch = 0;
    i = min_lag_8kHz;
    while i < max_lag_8kHz + 1 {
        if d_comp[(i + 1) as usize] > 0 {
            d_srch[length_d_srch as usize] = i;
            length_d_srch += 1;
        }
        i += 1;
    }

    /* Convolution */
    i = max_lag_8kHz + 3;
    while i >= min_lag_8kHz {
        d_comp[i as usize] = dcomp_add(
            d_comp[i as usize],
            d_comp[(i - 1) as usize] as opus_int32
                + d_comp[(i - 2) as usize] as opus_int32
                + d_comp[(i - 3) as usize] as opus_int32,
        );
        i -= 1;
    }

    length_d_comp = 0;
    i = min_lag_8kHz;
    while i < max_lag_8kHz + 4 {
        if d_comp[i as usize] > 0 {
            d_comp[length_d_comp as usize] = (i - 2) as opus_int16;
            length_d_comp += 1;
        }
        i += 1;
    }

    /**********************************************************************************
    ** SECOND STAGE, operating at 8 kHz, on lag sections with high correlation
    *************************************************************************************/
    /*********************************************************************************
    * Find energy of each subframe projected onto its history, for a range of delays
    *********************************************************************************/
    for row in C.iter_mut() {
        for v in row.iter_mut() {
            *v = 0.0;
        }
    }

    /* `target_ptr` selects between `frame` (Fs_kHz == 8) and `frame_8kHz` */
    let mut target_idx: usize = (PE_LTP_MEM_LENGTH_MS * 8) as usize;
    let frame_ref: &[silk_float] = if Fs_kHz == 8 { frame } else { &frame_8kHz };
    k = 0;
    while k < nb_subfr {
        energy_tmp = silk_energy_FLP(&frame_ref[target_idx..], sf_length_8kHz) + 1.0;
        j = 0;
        while j < length_d_comp {
            d = d_comp[j as usize] as opus_int;
            let basis_idx = target_idx - d as usize;
            cross_corr = silk_inner_product_FLP(
                &frame_ref[basis_idx..],
                &frame_ref[target_idx..],
                sf_length_8kHz,
                arch,
            );
            if cross_corr > 0.0f64 {
                energy = silk_energy_FLP(&frame_ref[basis_idx..], sf_length_8kHz);
                C[k as usize][d as usize] = (2.0f64 * cross_corr / (energy + energy_tmp)) as silk_float;
            } else {
                C[k as usize][d as usize] = 0.0f32;
            }
            j += 1;
        }
        target_idx += sf_length_8kHz as usize;
        k += 1;
    }

    /* search over lag range and lags codebook */
    /* scale factor for lag codebook, as a function of center lag */

    CCmax = 0.0f32; /* This value doesn't matter */
    CCmax_b = -1000.0f32;

    CBimax = 0; /* To avoid returning undefined lag values */
    lag = -1; /* To check if lag with strong enough correlation has been found */

    if prevLag > 0 {
        if Fs_kHz == 12 {
            prevLag = silk_LSHIFT(prevLag, 1) / 3;
        } else if Fs_kHz == 16 {
            prevLag = silk_RSHIFT(prevLag, 1);
        }
        prevLag_log2 = silk_log2((prevLag as silk_float) as f64);
    } else {
        prevLag_log2 = 0.0;
    }

    /* Set up stage 2 codebook based on number of subframes */
    if nb_subfr == PE_MAX_NB_SUBFR {
        cbk_size = PE_NB_CBKS_STAGE2_EXT;
        lag_cb = LagCb::Stage2;
        if Fs_kHz == 8 && complexity > SILK_PE_MIN_COMPLEX {
            /* If input is 8 khz use a larger codebook here because it is last stage */
            nb_cbk_search = PE_NB_CBKS_STAGE2_EXT;
        } else {
            nb_cbk_search = PE_NB_CBKS_STAGE2;
        }
    } else {
        cbk_size = PE_NB_CBKS_STAGE2_10MS;
        lag_cb = LagCb::Stage2_10ms;
        nb_cbk_search = PE_NB_CBKS_STAGE2_10MS;
    }

    k = 0;
    while k < length_d_srch {
        d = d_srch[k as usize];
        j = 0;
        while j < nb_cbk_search {
            CC[j as usize] = 0.0f32;
            i = 0;
            while i < nb_subfr {
                /* Try all codebooks */
                CC[j as usize] += C[i as usize][(d + lag_cb_val(lag_cb, i, j)) as usize];
                i += 1;
            }
            j += 1;
        }
        /* Find best codebook */
        CCmax_new = -1000.0f32;
        CBimax_new = 0;
        i = 0;
        while i < nb_cbk_search {
            if CC[i as usize] > CCmax_new {
                CCmax_new = CC[i as usize];
                CBimax_new = i;
            }
            i += 1;
        }

        /* Bias towards shorter lags */
        lag_log2 = silk_log2((d as silk_float) as f64);
        CCmax_new_b = CCmax_new - PE_SHORTLAG_BIAS * nb_subfr as silk_float * lag_log2;

        /* Bias towards previous lag */
        if prevLag > 0 {
            delta_lag_log2_sqr = lag_log2 - prevLag_log2;
            delta_lag_log2_sqr *= delta_lag_log2_sqr;
            CCmax_new_b -= PE_PREVLAG_BIAS
                * nb_subfr as silk_float
                * (*LTPCorr)
                * delta_lag_log2_sqr
                / (delta_lag_log2_sqr + 0.5f32);
        }

        if CCmax_new_b > CCmax_b && /* Find maximum biased correlation */
            CCmax_new > nb_subfr as silk_float * search_thres2
        /* Correlation needs to be high enough to be voiced */
        {
            CCmax_b = CCmax_new_b;
            CCmax = CCmax_new;
            lag = d;
            CBimax = CBimax_new;
        }
        k += 1;
    }

    if lag == -1 {
        /* No suitable candidate found */
        for v in pitch_out.iter_mut().take(PE_MAX_NB_SUBFR as usize) {
            *v = 0;
        }
        *LTPCorr = 0.0;
        *lagIndex = 0;
        *contourIndex = 0;
        return 1;
    }

    /* Output normalized correlation */
    *LTPCorr = (CCmax / nb_subfr as silk_float) as silk_float;
    silk_assert!(*LTPCorr >= 0.0f32);

    if Fs_kHz > 8 {
        /* Search in original signal */

        /* Compensate for decimation */
        silk_assert!(lag == silk_SAT16(lag));
        if Fs_kHz == 12 {
            lag = silk_RSHIFT_ROUND(silk_SMULBB(lag, 3), 1);
        } else {
            /* Fs_kHz == 16 */
            lag = silk_LSHIFT(lag, 1);
        }

        lag = silk_LIMIT(lag, min_lag, max_lag);
        start_lag = silk_max_int(lag - 2, min_lag);
        end_lag = silk_min_int(lag + 2, max_lag);
        lag_new = lag; /* to avoid undefined lag */
        CBimax = 0; /* to avoid undefined lag */

        CCmax = -1000.0f32;

        /* Calculate the correlations and energies needed in stage 3 */
        silk_P_Ana_calc_corr_st3(
            &mut cross_corr_st3,
            frame,
            start_lag,
            sf_length,
            nb_subfr,
            complexity,
        );
        silk_P_Ana_calc_energy_st3(
            &mut energies_st3,
            frame,
            start_lag,
            sf_length,
            nb_subfr,
            complexity,
        );

        lag_counter = 0;
        silk_assert!(lag == silk_SAT16(lag));
        contour_bias = PE_FLATCONTOUR_BIAS / lag as silk_float;

        /* Set up cbk parameters according to complexity setting and frame length */
        if nb_subfr == PE_MAX_NB_SUBFR {
            nb_cbk_search = silk_nb_cbk_searchs_stage3[complexity as usize] as opus_int;
            cbk_size = PE_NB_CBKS_STAGE3_MAX;
            lag_cb = LagCb::Stage3;
        } else {
            nb_cbk_search = PE_NB_CBKS_STAGE3_10MS;
            cbk_size = PE_NB_CBKS_STAGE3_10MS;
            lag_cb = LagCb::Stage3_10ms;
        }
        let _ = cbk_size;

        target_idx = (PE_LTP_MEM_LENGTH_MS * Fs_kHz) as usize;
        energy_tmp = silk_energy_FLP(&frame[target_idx..], nb_subfr * sf_length) + 1.0;
        d = start_lag;
        while d <= end_lag {
            j = 0;
            while j < nb_cbk_search {
                cross_corr = 0.0;
                energy = energy_tmp;
                k = 0;
                while k < nb_subfr {
                    cross_corr +=
                        cross_corr_st3[k as usize][j as usize][lag_counter as usize] as f64;
                    energy += energies_st3[k as usize][j as usize][lag_counter as usize] as f64;
                    k += 1;
                }
                if cross_corr > 0.0 {
                    CCmax_new = (2.0f64 * cross_corr / energy) as silk_float;
                    /* Reduce depending on flatness of contour */
                    CCmax_new *= 1.0f32 - contour_bias * j as silk_float;
                } else {
                    CCmax_new = 0.0f32;
                }

                if CCmax_new > CCmax
                    && (d + silk_CB_lags_stage3[0][j as usize] as opus_int) <= max_lag
                {
                    CCmax = CCmax_new;
                    lag_new = d;
                    CBimax = j;
                }
                j += 1;
            }
            lag_counter += 1;
            d += 1;
        }

        k = 0;
        while k < nb_subfr {
            pitch_out[k as usize] = lag_new + lag_cb_val(lag_cb, k, CBimax);
            pitch_out[k as usize] = silk_LIMIT(pitch_out[k as usize], min_lag, PE_MAX_LAG_MS * Fs_kHz);
            k += 1;
        }
        *lagIndex = (lag_new - min_lag) as opus_int16;
        *contourIndex = CBimax as opus_int8;
    } else {
        /* Fs_kHz == 8 */
        /* Save Lags */
        k = 0;
        while k < nb_subfr {
            pitch_out[k as usize] = lag + lag_cb_val(lag_cb, k, CBimax);
            pitch_out[k as usize] = silk_LIMIT(pitch_out[k as usize], min_lag_8kHz, PE_MAX_LAG_MS * 8);
            k += 1;
        }
        *lagIndex = (lag - min_lag_8kHz) as opus_int16;
        *contourIndex = CBimax as opus_int8;
    }
    celt_assert!(*lagIndex >= 0);
    /* return as voiced */
    0
}

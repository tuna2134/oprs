/* Copyright (c) 2007-2008 CSIRO
   Copyright (c) 2007-2009 Xiph.Org Foundation
   Copyright (c) 2008-2009 Gregory Maxwell
   Written by Jean-Marc Valin and Gregory Maxwell */
/*
   Redistribution and use in source and binary forms, with or without
   modification, are permitted provided that the following conditions
   are met:

   - Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

   - Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.

   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
   ``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
   LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
   A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
   OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
   EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
   PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
   PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
   LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
   NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
   SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/

//! Transliteration of `celt/bands.h` + `celt/bands.c` (libopus 1.6.1) into
//! safe Rust.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! Configuration: floating point (`FIXED_POINT` undefined), `FLOAT_APPROX`
//! undefined, x86 intrinsics + RTCD enabled (host arch 4).  `ENABLE_QEXT`,
//! `ENABLE_RES24`, `DISABLE_UPDATE_DRAFT`, `RESYNTH` and `FUZZING` are **not**
//! defined; `ENABLE_DEEP_PLC`/`DRED`/`OSCE`/`CUSTOM_MODES`/`ENABLE_HARDENING`
//! are, but change nothing in this file.  Every `#ifdef ENABLE_QEXT`,
//! `#ifdef FIXED_POINT`, `#ifdef DISABLE_UPDATE_DRAFT` and `#ifdef RESYNTH`
//! branch that is not compiled in the reference is therefore dropped.
//!
//! `exp_rotation`/`exp_rotation1`/`alg_quant`/`alg_unquant`/
//! `renormalise_vector`/`stereo_itheta` are imported from [`super::vq`],
//! `bits2pulses`/`pulses2bits`/`get_pulses`/`QTHETA_OFFSET*` from
//! [`super::rate`], `celt_inner_prod` from [`super::kernels`], and
//! `bitexact_cos`/`bitexact_log2tan`/`celt_lcg_rand` from [`super::mathops`]
//! (their C definitions live in `bands.c` but they were verified as part of
//! the `mathops` probe and are re-exported here under their C names).
//!
//! ## Raw pointers
//!
//! `quant_all_bands()` and everything below it alias their buffers through raw
//! pointers exactly like the C: `X`, `Y`, `lowband`, `lowband_out` and
//! `lowband_scratch` are all *views* that may point into the caller's `X_`/`Y_`
//! arrays or into the internal `norm`/`norm2` scratch, and `quant_band()`
//! deliberately runs `haar1()`/`deinterleave_hadamard()` on `lowband_scratch`
//! after copying into it (so a copy into a view that aliases `X` transforms
//! `X` twice).  That aliasing is part of the bitstream semantics, so the
//! transliteration keeps `*mut celt_norm` and the identical pointer
//! arithmetic; `unsafe` is confined to creating those views.  The public entry
//! points take ordinary slices.

#![allow(unsafe_op_in_unsafe_fn)]

use super::arch::*;
use super::celt::{MAX_CELT_FRAME, MAX_CHANNELS, MAX_PVQ_N};
use super::entcode::{celt_sudiv, celt_udiv, ec_ctx, BITRES};
use super::entdec::{ec_dec_bit_logp, ec_dec_bits, ec_dec_uint, ec_decode, ec_dec_update};
use super::entenc::{ec_encode, ec_enc_bit_logp, ec_enc_bits, ec_enc_uint};
use super::kernels::celt_inner_prod;
use super::mathops::{
    celt_exp2, celt_exp2_db, celt_rsqrt, celt_rsqrt_norm32, celt_sqrt, isqrt32, FRAC_MUL16,
};
use super::modes::CELTMode;
use super::quant_bands::eMeans;
use super::rate::{get_pulses, QTHETA_OFFSET, QTHETA_OFFSET_TWOPHASE};
use super::vq::{alg_quant, alg_unquant, renormalise_vector};
use crate::types::*;

/// `bands.c` defines `bitexact_cos`, `bitexact_log2tan` and `celt_lcg_rand`;
/// they were verified as part of the `mathops` probe and live there.  They are
/// re-exported here so `crate::celt::bands::bitexact_cos` keeps working.
pub use super::mathops::{bitexact_cos, bitexact_log2tan, celt_lcg_rand};

/* ------------------------------------------------------------------------- *
 * helpers for the raw-pointer views
 * ------------------------------------------------------------------------- */

#[inline(always)]
unsafe fn als<'a, T>(p: *const T, n: i32) -> &'a [T] {
    core::slice::from_raw_parts(p, n as usize)
}

#[inline(always)]
unsafe fn alms<'a, T>(p: *mut T, n: i32) -> &'a mut [T] {
    core::slice::from_raw_parts_mut(p, n as usize)
}

/// `bands.h`: `SPREAD_NONE`
pub const SPREAD_NONE: i32 = 0;
/// `bands.h`: `SPREAD_LIGHT`
pub const SPREAD_LIGHT: i32 = 1;
/// `bands.h`: `SPREAD_NORMAL`
pub const SPREAD_NORMAL: i32 = 2;
/// `bands.h`: `SPREAD_AGGRESSIVE`
pub const SPREAD_AGGRESSIVE: i32 = 3;

/// `bands.c`: `#define MIN_STEREO_ENERGY 1e-10f` (float build)
const MIN_STEREO_ENERGY: opus_val32 = 1e-10f32;

// ---------------------------------------------------------------------------
// bands.c — the functions proper
// ---------------------------------------------------------------------------

/// `hysteresis_decision()`
pub fn hysteresis_decision(
    val: opus_val16,
    thresholds: &[opus_val16],
    hysteresis: &[opus_val16],
    N: i32,
    prev: i32,
) -> i32 {
    let mut i: i32 = 0;
    while i < N {
        if val < thresholds[i as usize] {
            break;
        }
        i += 1;
    }
    if i > prev && val < thresholds[prev as usize] + hysteresis[prev as usize] {
        i = prev;
    }
    if i < prev && val > thresholds[prev as usize - 1] - hysteresis[prev as usize - 1] {
        i = prev;
    }
    i
}

/* Compute the amplitude (sqrt energy) in each of the bands */
/// `compute_band_energies()` — float body (`FIXED_POINT` undefined).
pub fn compute_band_energies(
    m: &CELTMode,
    X: &[celt_sig],
    bandE: &mut [celt_ener],
    end: i32,
    C: i32,
    LM: i32,
    arch: i32,
) {
    let _ = arch;
    let eBands = &m.eBands;
    let N: i32 = m.shortMdctSize << LM;
    for c in 0..C {
        for i in 0..end {
            let off: i32 = c * N + ((eBands[i as usize] as i32) << LM);
            let len: i32 = ((eBands[i as usize + 1] as i32) - (eBands[i as usize] as i32)) << LM;
            /* `sum = 1e-27f + celt_inner_prod(&X[..], &X[..], len, arch);` — the
               reference dispatches to `celt_inner_prod_sse`. */
            let sum = 1e-27f32 + celt_inner_prod(&X[off as usize..], &X[off as usize..], len);
            bandE[(i + c * m.nbEBands) as usize] = celt_sqrt(sum);
        }
    }
}

/* Normalise each band such that the energy is one. */
/// `normalise_bands()` — float body (`FIXED_POINT` undefined).
pub fn normalise_bands(
    m: &CELTMode,
    freq: &[celt_sig],
    X: &mut [celt_norm],
    bandE: &[celt_ener],
    end: i32,
    C: i32,
    M: i32,
) {
    let eBands = &m.eBands;
    let N: i32 = M * m.shortMdctSize;
    for c in 0..C {
        for i in 0..end {
            let g: opus_val16 = 1.0f32 / (1e-27f32 + bandE[(i + c * m.nbEBands) as usize]);
            let mut j: i32 = M * eBands[i as usize] as i32;
            while j < M * eBands[i as usize + 1] as i32 {
                X[(j + c * N) as usize] = freq[(j + c * N) as usize] * g;
                j += 1;
            }
        }
    }
}

/* De-normalise the energy to produce the synthesis from the unit-energy bands */
/// `denormalise_bands()` — float body (`FIXED_POINT` undefined).
pub fn denormalise_bands(
    m: &CELTMode,
    X: &[celt_norm],
    freq: &mut [celt_sig],
    bandLogE: &[celt_glog],
    start: i32,
    end: i32,
    M: i32,
    downsample: i32,
    silence: i32,
) {
    let mut start = start;
    let mut end = end;
    let eBands = &m.eBands;
    let N: i32 = M * m.shortMdctSize;
    let mut bound: i32 = M * eBands[end as usize] as i32;
    if downsample != 1 {
        bound = IMIN(bound, N / downsample);
    }
    if silence != 0 {
        bound = 0;
        start = 0;
        end = 0;
    }
    let mut f: i32 = M * eBands[start as usize] as i32;
    if start != 0 {
        let mut i: i32 = 0;
        while i < M * eBands[start as usize] as i32 {
            freq[i as usize] = 0.0f32;
            i += 1;
        }
    }
    for i in start..end {
        let band_end: i32 = M * eBands[i as usize + 1] as i32;
        let mut j: i32 = M * eBands[i as usize] as i32;
        /* `lg = ADD32(bandLogE[i], SHL32((opus_val32)eMeans[i], DB_SHIFT-4));` —
           `DB_SHIFT` is 24 and `SHL32` is the identity in the float build. */
        let lg: celt_glog = ADD32(
            bandLogE[i as usize],
            SHL32(eMeans[i as usize] as opus_val32, 24 - 4),
        );
        let g: opus_val32 = celt_exp2_db(MIN32(32.0f32, lg));
        while j < band_end {
            /* `*f++ = PSHR32(MULT32_32_Q31(SHL32(*x, 30-NORM_SHIFT), g), shift);`
               with `x == X + M*eBands[start]`; in the float build every shift
               and `PSHR32` is the identity, and `f == j`. */
            freq[f as usize] = MULT32_32_Q31(SHL32(X[j as usize], 30 - NORM_SHIFT), g);
            f += 1;
            j += 1;
        }
    }
    celt_assert!(start <= end);
    for k in bound..N {
        freq[k as usize] = 0.0f32;
    }
}

/* This prevents energy collapse for transients with multiple short MDCTs */
/// `anti_collapse()` — float body (`FIXED_POINT` undefined).
pub fn anti_collapse(
    m: &CELTMode,
    X_: &mut [celt_norm],
    collapse_masks: &[u8],
    LM: i32,
    C: i32,
    size: i32,
    start: i32,
    end: i32,
    logE: &[celt_glog],
    prev1logE: &[celt_glog],
    prev2logE: &[celt_glog],
    pulses: &[i32],
    seed_in: opus_uint32,
    encode: i32,
    arch: i32,
) {
    let eBands = &m.eBands;
    let nbe = m.nbEBands;
    let mut seed = seed_in;
    let xbase = X_.as_mut_ptr();
    for i in start..end {
        let N0: i32 = eBands[i as usize + 1] as i32 - eBands[i as usize] as i32;
        /* depth in 1/8 bits */
        celt_sig_assert!(pulses[i as usize] >= 0);
        let depth: i32 = (celt_udiv(
            (1 + pulses[i as usize]) as opus_uint32,
            (eBands[i as usize + 1] as i32 - eBands[i as usize] as i32) as opus_uint32,
        ) >> LM) as i32;

        let thresh: opus_val16 = 0.5f32 * celt_exp2(-0.125f32 * depth as opus_val16);
        let sqrt_1: opus_val32 = celt_rsqrt((N0 as opus_val32) * (1 << LM) as opus_val32);

        for c in 0..C {
            let mut prev1: celt_glog = prev1logE[(c * nbe + i) as usize];
            let mut prev2: celt_glog = prev2logE[(c * nbe + i) as usize];
            if encode == 0 && C == 1 {
                prev1 = MAX32(prev1, prev1logE[(nbe + i) as usize]);
                prev2 = MAX32(prev2, prev2logE[(nbe + i) as usize]);
            }
            let mut Ediff: opus_val32 = logE[(c * nbe + i) as usize] - MIN32(prev1, prev2);
            Ediff = MAX32(0.0f32, Ediff);

            /* r needs to be multiplied by 2 or 2*sqrt(2) depending on LM because
               short blocks don't have the same energy as long */
            let mut r: opus_val16 = 2.0f32 * celt_exp2_db(-Ediff);
            if LM == 3 {
                r *= 1.41421356f32;
            }
            r = MIN16(thresh, r);
            r = r * sqrt_1;
            /* `X = X_ + c*size + (m->eBands[i]<<LM);` */
            let X = unsafe { xbase.add((c * size + ((eBands[i as usize] as i32) << LM)) as usize) };
            let mut renormalize: i32 = 0;
            for k in 0..(1i32 << LM) {
                /* Detect collapse */
                if (collapse_masks[(i * C + c) as usize] as i32) & (1 << k) == 0 {
                    /* Fill with noise */
                    for j in 0..N0 {
                        seed = celt_lcg_rand(seed);
                        unsafe {
                            *X.add(((j << LM) + k) as usize) =
                                if (seed & 0x8000) != 0 { r } else { -r };
                        }
                    }
                    renormalize = 1;
                }
            }
            /* We just added some energy, so we need to renormalise */
            if renormalize != 0 {
                renormalise_vector(unsafe { alms(X, N0 << LM) }, N0 << LM, Q31ONE, arch);
            }
        }
    }
}

/* Compute the weights to use for optimizing normalized distortion across
   channels. We use the amplitude to weight square distortion, which means
   that we use the square root of the value we would have been using if we
   wanted to minimize the MSE in the non-normalized domain. This roughly
   corresponds to some quick-and-dirty perceptual experiments I ran to
   measure inter-aural masking (there doesn't seem to be any published data
   on the topic). */
/// `compute_channel_weights()` (`static` in C)
pub fn compute_channel_weights(Ex: celt_ener, Ey: celt_ener, w: &mut [opus_val16; 2]) {
    let minE: celt_ener = MIN32(Ex, Ey);
    /* Adjustment to make the weights a bit more conservative. */
    let Ex = ADD32(Ex, minE / 3.0f32);
    let Ey = ADD32(Ey, minE / 3.0f32);
    /* `w[0] = VSHR32(Ex, shift); w[1] = VSHR32(Ey, shift);` — float: identity. */
    w[0] = VSHR32(Ex, 0);
    w[1] = VSHR32(Ey, 0);
}

/// `intensity_stereo()` (`static` in C)
pub fn intensity_stereo(
    m: &CELTMode,
    X: &mut [celt_norm],
    Y: &[celt_norm],
    bandE: &[celt_ener],
    bandID: i32,
    N: i32,
) {
    let i = bandID;
    let left: opus_val16 = VSHR32(bandE[i as usize], 0);
    let right: opus_val16 = VSHR32(bandE[(i + m.nbEBands) as usize], 0);
    let norm: opus_val16 =
        EPSILON + celt_sqrt(EPSILON + MULT16_16(left, left) + MULT16_16(right, right));
    let a1: opus_val16 = DIV32_16(SHL32(EXTEND32(left), 15), norm);
    let a2: opus_val16 = DIV32_16(SHL32(EXTEND32(right), 15), norm);
    for j in 0..N as usize {
        X[j] = ADD32(MULT16_32_Q15(a1, X[j]), MULT16_32_Q15(a2, Y[j]));
    }
}

/// `stereo_split()` (`static` in C)
pub fn stereo_split(X: &mut [celt_norm], Y: &mut [celt_norm], N: i32) {
    for j in 0..N as usize {
        let l: opus_val32 = MULT32_32_Q31(QCONST32(0.70710678f32, 31), X[j]);
        let r: opus_val32 = MULT32_32_Q31(QCONST32(0.70710678f32, 31), Y[j]);
        X[j] = ADD32(l, r);
        Y[j] = SUB32(r, l);
    }
}

/// `stereo_merge()` (`static` in C)
pub fn stereo_merge(
    X: &mut [celt_norm],
    Y: &mut [celt_norm],
    mid: opus_val32,
    N: i32,
    arch: i32,
) {
    let _ = arch;
    /* Compute the norm of X+Y and X-Y as |X|^2 + |Y|^2 +/- sum(xy) */
    /* `celt_inner_prod_norm_shift` is `celt_inner_prod` in the float build. */
    let xp0: opus_val32 = celt_inner_prod(Y, X, N);
    let side: opus_val32 = celt_inner_prod(Y, Y, N);
    /* Compensating for the mid normalization */
    let xp: opus_val32 = MULT32_32_Q31(mid, xp0);
    /* mid and side are in Q15, not Q14 like X and Y */
    let El: opus_val32 = SHR32(MULT32_32_Q31(mid, mid), 3) + side - 2.0f32 * xp;
    let Er: opus_val32 = SHR32(MULT32_32_Q31(mid, mid), 3) + side + 2.0f32 * xp;
    if Er < QCONST32(6e-4f32, 28) || El < QCONST32(6e-4f32, 28) {
        Y[..N as usize].copy_from_slice(&X[..N as usize]);
        return;
    }

    let lgain: opus_val32 = celt_rsqrt_norm32(VSHR32(El, 0));
    let rgain: opus_val32 = celt_rsqrt_norm32(VSHR32(Er, 0));

    for j in 0..N as usize {
        /* Apply mid scaling (side is already scaled) */
        let l: celt_norm = MULT32_32_Q31(mid, X[j]);
        let r: celt_norm = Y[j];
        X[j] = VSHR32(MULT32_32_Q31(lgain, SUB32(l, r)), 0);
        Y[j] = VSHR32(MULT32_32_Q31(rgain, ADD32(l, r)), 0);
    }
}

/* Decide whether we should spread the pulses in the current frame */
/// `spreading_decision()`
pub fn spreading_decision(
    m: &CELTMode,
    X: &[celt_norm],
    average: &mut i32,
    last_decision: i32,
    hf_average: &mut i32,
    tapset_decision: &mut i32,
    update_hf: i32,
    end: i32,
    C: i32,
    M: i32,
    spread_weight: &[i32],
) -> i32 {
    let i: i32;
    let c: i32;
    let N0: i32;
    let mut sum: i32 = 0;
    let mut nbBands: i32 = 0;
    let eBands = &m.eBands;
    let decision: i32;
    let mut hf_sum: i32 = 0;

    celt_assert!(end > 0);

    N0 = M * m.shortMdctSize;

    if M * (eBands[end as usize] as i32 - eBands[end as usize - 1] as i32) <= 8 {
        return SPREAD_NONE;
    }
    for c in 0..C {
        for i in 0..end {
            let mut j: i32;
            let N: i32;
            let mut tmp: i32 = 0;
            let mut tcount: [i32; 3] = [0, 0, 0];
            let x: i32 = M * eBands[i as usize] as i32 + c * N0;
            N = M * (eBands[i as usize + 1] as i32 - eBands[i as usize] as i32);
            if N <= 8 {
                continue;
            }
            /* Compute rough CDF of |x[j]| */
            for j in 0..N {
                /* `x2N = MULT16_16(MULT16_16_Q15(SHR32(x[j], NORM_SHIFT-14),
                   SHR32(x[j], NORM_SHIFT-14)), N);` — float: (x*x)*N */
                let x2N: opus_val32 = MULT16_16(
                    MULT16_16_Q15(
                        SHR32(X[(x + j) as usize], NORM_SHIFT - 14),
                        SHR32(X[(x + j) as usize], NORM_SHIFT - 14),
                    ),
                    N as opus_val32,
                );
                if x2N < QCONST16(0.25f32, 13) {
                    tcount[0] += 1;
                }
                if x2N < QCONST16(0.0625f32, 13) {
                    tcount[1] += 1;
                }
                if x2N < QCONST16(0.015625f32, 13) {
                    tcount[2] += 1;
                }
            }

            /* Only include four last bands (8 kHz and up) */
            if i > m.nbEBands - 4 {
                hf_sum +=
                    celt_udiv((32 * (tcount[1] + tcount[0])) as opus_uint32, N as opus_uint32) as i32;
            }
            tmp = (2 * tcount[2] >= N) as i32
                + (2 * tcount[1] >= N) as i32
                + (2 * tcount[0] >= N) as i32;
            sum += tmp * spread_weight[i as usize];
            nbBands += spread_weight[i as usize];
        }
    }

    if update_hf != 0 {
        if hf_sum != 0 {
            hf_sum =
                celt_udiv(hf_sum as opus_uint32, (C * (4 - m.nbEBands + end)) as opus_uint32) as i32;
        }
        *hf_average = (*hf_average + hf_sum) >> 1;
        hf_sum = *hf_average;
        if *tapset_decision == 2 {
            hf_sum += 4;
        } else if *tapset_decision == 0 {
            hf_sum -= 4;
        }
        if hf_sum > 22 {
            *tapset_decision = 2;
        } else if hf_sum > 18 {
            *tapset_decision = 1;
        } else {
            *tapset_decision = 0;
        }
    }
    celt_assert!(nbBands > 0); /* end has to be non-zero */
    celt_assert!(sum >= 0);
    sum = celt_udiv(((sum as opus_int32) << 8) as opus_uint32, nbBands as opus_uint32) as i32;
    /* Recursive averaging */
    sum = (sum + *average) >> 1;
    *average = sum;
    /* Hysteresis */
    sum = (3 * sum + (((3 - last_decision) << 7) + 64) + 2) >> 2;
    if sum < 80 {
        decision = SPREAD_AGGRESSIVE;
    } else if sum < 256 {
        decision = SPREAD_NORMAL;
    } else if sum < 384 {
        decision = SPREAD_LIGHT;
    } else {
        decision = SPREAD_NONE;
    }
    decision
}

/* Indexing table for converting from natural Hadamard to ordery Hadamard
   This is essentially a bit-reversed Gray, on top of which we've added
   an inversion of the order because we want the DC at the end rather than
   the beginning. The lines are for N=2, 4, 8, 16 */
static ordery_table: [i32; 30] = [
    1, 0, 3, 0, 2, 1, 7, 0, 4, 3, 6, 1, 5, 2, 15, 0, 8, 7, 12, 3, 11, 4, 14, 1, 9, 6, 13, 2, 10,
    5,
];

/// `deinterleave_hadamard()` (`static` in C)
pub fn deinterleave_hadamard(X: &mut [celt_norm], N0: i32, stride: i32, hadamard: i32) {
    let i: i32;
    let j: i32;
    let N: i32 = N0 * stride;
    /* `VARDECL(celt_norm, tmp); ALLOC(tmp, N, celt_norm);` — `N` is one PVQ
       band in `quant_band`, so `MAX_PVQ_N` covers the codec; the `bands` probe
       calls this directly for `N` up to 512 and falls back to the heap. */
    let mut tmp_heap: Vec<celt_norm> = Vec::new();
    let mut tmp_storage = [0.0f32; MAX_PVQ_N];
    let tmp: &mut [celt_norm] = if N as usize <= MAX_PVQ_N {
        &mut tmp_storage[..N as usize]
    } else {
        tmp_heap.resize(N as usize, 0.0);
        tmp_heap.as_mut_slice()
    };
    celt_assert!(stride > 0);
    if hadamard != 0 {
        let ordery_off = (stride - 2) as usize;
        for i in 0..stride {
            for j in 0..N0 {
                tmp[(ordery_table[ordery_off + i as usize] * N0 + j) as usize] =
                    X[(j * stride + i) as usize];
            }
        }
    } else {
        for i in 0..stride {
            for j in 0..N0 {
                tmp[(i * N0 + j) as usize] = X[(j * stride + i) as usize];
            }
        }
    }
    X[..N as usize].copy_from_slice(&tmp[..N as usize]);
}

/// `interleave_hadamard()` (`static` in C)
pub fn interleave_hadamard(X: &mut [celt_norm], N0: i32, stride: i32, hadamard: i32) {
    let i: i32;
    let j: i32;
    let N: i32 = N0 * stride;
    /* `VARDECL(celt_norm, tmp); ALLOC(tmp, N, celt_norm);` — `N` is one PVQ
       band in `quant_band`, so `MAX_PVQ_N` covers the codec; the `bands` probe
       calls this directly for `N` up to 512 and falls back to the heap. */
    let mut tmp_heap: Vec<celt_norm> = Vec::new();
    let mut tmp_storage = [0.0f32; MAX_PVQ_N];
    let tmp: &mut [celt_norm] = if N as usize <= MAX_PVQ_N {
        &mut tmp_storage[..N as usize]
    } else {
        tmp_heap.resize(N as usize, 0.0);
        tmp_heap.as_mut_slice()
    };
    if hadamard != 0 {
        let ordery_off = (stride - 2) as usize;
        for i in 0..stride {
            for j in 0..N0 {
                tmp[(j * stride + i) as usize] =
                    X[(ordery_table[ordery_off + i as usize] * N0 + j) as usize];
            }
        }
    } else {
        for i in 0..stride {
            for j in 0..N0 {
                tmp[(j * stride + i) as usize] = X[(i * N0 + j) as usize];
            }
        }
    }
    X[..N as usize].copy_from_slice(&tmp[..N as usize]);
}

/// `haar1()`
pub fn haar1(X: &mut [celt_norm], N0_in: i32, stride: i32) {
    let i: i32;
    let j: i32;
    let N0: i32 = N0_in >> 1;
    for i in 0..stride {
        for j in 0..N0 {
            let tmp1: opus_val32 =
                MULT32_32_Q31(QCONST32(0.70710678f32, 31), X[(stride * 2 * j + i) as usize]);
            let tmp2: opus_val32 = MULT32_32_Q31(
                QCONST32(0.70710678f32, 31),
                X[(stride * (2 * j + 1) + i) as usize],
            );
            X[(stride * 2 * j + i) as usize] = ADD32(tmp1, tmp2);
            X[(stride * (2 * j + 1) + i) as usize] = SUB32(tmp1, tmp2);
        }
    }
}

/// `compute_qn()` (`static` in C)
fn compute_qn(N: i32, b: i32, offset: i32, pulse_cap: i32, stereo: i32) -> i32 {
    static exp2_table8: [opus_int16; 8] = [16384, 17866, 19483, 21247, 23170, 25267, 27554, 30048];
    let mut qn: i32;
    let mut qb: i32;
    let mut N2: i32 = 2 * N - 1;
    if stereo != 0 && N == 2 {
        N2 -= 1;
    }
    /* The upper limit ensures that in a stereo split with itheta==16384, we'll
       always have enough bits left over to code at least one pulse in the
       side; otherwise it would collapse, since it doesn't get folded. */
    qb = celt_sudiv(b + N2 * offset, N2);
    qb = IMIN(b - pulse_cap - (4 << BITRES), qb);

    qb = IMIN(8 << BITRES, qb);

    if qb < (1 << BITRES >> 1) {
        qn = 1;
    } else {
        qn = (exp2_table8[(qb & 0x7) as usize] as i32) >> (14 - (qb >> BITRES));
        qn = (qn + 1) >> 1 << 1;
    }
    celt_assert!(qn <= 256);
    qn
}

/// `struct band_ctx` (`static` in C) — the `ENABLE_QEXT` fields are dropped.
#[derive(Clone, Copy)]
struct band_ctx<'a> {
    encode: i32,
    resynth: i32,
    m: &'a CELTMode,
    i: i32,
    intensity: i32,
    spread: i32,
    tf_change: i32,
    ec: *mut ec_ctx,
    remaining_bits: opus_int32,
    bandE: &'a [celt_ener],
    seed: opus_uint32,
    arch: i32,
    theta_round: i32,
    disable_inv: i32,
    avoid_split_noise: i32,
}

/// `struct split_ctx` (`static` in C) — the `ENABLE_QEXT` field is dropped.
#[derive(Clone, Copy, Default)]
struct split_ctx {
    inv: i32,
    imid: i32,
    iside: i32,
    delta: i32,
    itheta: i32,
    qalloc: i32,
}

/// `compute_theta()` (`static` in C) — float, non-`ENABLE_QEXT` body.
unsafe fn compute_theta(
    ctx: &band_ctx,
    sctx: &mut split_ctx,
    X: *mut celt_norm,
    Y: *mut celt_norm,
    N: i32,
    b: &mut i32,
    B: i32,
    B0: i32,
    LM: i32,
    stereo: i32,
    fill: &mut i32,
) {
    let mut itheta: i32 = 0;
    let mut delta: i32 = 0;
    let imid: i32;
    let iside: i32;
    let qalloc: i32;
    let pulse_cap: i32;
    let offset: i32;
    let tell: opus_uint32;
    let mut inv: i32 = 0;
    let encode: i32 = ctx.encode;
    let m: &CELTMode = ctx.m;
    let i: i32 = ctx.i;
    let intensity: i32 = ctx.intensity;
    let ec: *mut ec_ctx = ctx.ec;
    let bandE: &[celt_ener] = ctx.bandE;

    /* Decide on the resolution to give to the split parameter theta */
    pulse_cap = m.logN[i as usize] as i32 + LM * (1 << BITRES);
    offset = (pulse_cap >> 1)
        - (if stereo != 0 && N == 2 {
            QTHETA_OFFSET_TWOPHASE
        } else {
            QTHETA_OFFSET
        });
    let mut qn = compute_qn(N, *b, offset, pulse_cap, stereo);
    if stereo != 0 && i >= intensity {
        qn = 1;
    }
    if encode != 0 {
        /* theta is the atan() of the ratio between the (normalized)
           side and mid. With just that parameter, we can re-scale both
           mid and side because we know that 1) they have unit norm and
           2) they are orthogonal. */
        let itheta_q30 = trace::stereo_itheta(als(X, N), als(Y, N), stereo, N, ctx.arch);
        itheta = itheta_q30 >> 16;
    }
    tell = trace::ec_tell_frac(744, &*ec);
    if qn != 1 {
        if encode != 0 {
            if stereo == 0 || ctx.theta_round == 0 {
                itheta = (itheta * qn + 8192) >> 14;
                if stereo == 0 && ctx.avoid_split_noise != 0 && itheta > 0 && itheta < qn {
                    /* Check if the selected value of theta will cause the bit allocation
                       to inject noise on one side. If so, make sure the energy of that side
                       is zero. */
                    let unquantized: i32 =
                        celt_udiv((itheta * 16384) as opus_uint32, qn as opus_uint32) as i32;
                    let imid_ = bitexact_cos(unquantized as opus_int16) as i32;
                    let iside_ = bitexact_cos((16384 - unquantized) as opus_int16) as i32;
                    delta = FRAC_MUL16((N - 1) << 7, bitexact_log2tan(iside_, imid_));
                    if delta > *b {
                        itheta = qn;
                    } else if delta < -*b {
                        itheta = 0;
                    }
                }
            } else {
                let down: i32;
                /* Bias quantization towards itheta=0 and itheta=16384. */
                let bias: i32 = if itheta > 8192 {
                    32767 / qn
                } else {
                    -32767 / qn
                };
                down = IMIN(qn - 1, IMAX(0, (itheta * qn + bias) >> 14));
                if ctx.theta_round < 0 {
                    itheta = down;
                } else {
                    itheta = down + 1;
                }
            }
        }
        /* Entropy coding of the angle. We use a uniform pdf for the
           time split, a step for stereo, and a triangular one for the rest. */
        if stereo != 0 && N > 2 {
            let p0: i32 = 3;
            let mut x: i32 = itheta;
            let x0: i32 = qn / 2;
            let ft: i32 = p0 * (x0 + 1) + x0;
            /* Use a probability of p0 up to itheta=8192 and then use 1 after */
            if encode != 0 {
                ec_encode(
                    &mut *ec,
                    (if x <= x0 { p0 * x } else { (x - 1 - x0) + (x0 + 1) * p0 }) as u32,
                    (if x <= x0 {
                        p0 * (x + 1)
                    } else {
                        (x - x0) + (x0 + 1) * p0
                    }) as u32,
                    ft as u32,
                );
            } else {
                let fs: i32;
                fs = ec_decode(&mut *ec, ft as u32) as i32;
                if fs < (x0 + 1) * p0 {
                    x = fs / p0;
                } else {
                    x = x0 + 1 + (fs - (x0 + 1) * p0);
                }
                ec_dec_update(
                    &mut *ec,
                    (if x <= x0 { p0 * x } else { (x - 1 - x0) + (x0 + 1) * p0 }) as u32,
                    (if x <= x0 {
                        p0 * (x + 1)
                    } else {
                        (x - x0) + (x0 + 1) * p0
                    }) as u32,
                    ft as u32,
                );
                itheta = x;
            }
        } else if B0 > 1 || stereo != 0 {
            /* Uniform pdf */
            if encode != 0 {
                ec_enc_uint(&mut *ec, itheta as u32, (qn + 1) as u32);
            } else {
                itheta = ec_dec_uint(&mut *ec, (qn + 1) as u32) as i32;
            }
        } else {
            let mut fs: i32 = 1;
            let ft: i32;
            ft = ((qn >> 1) + 1) * ((qn >> 1) + 1);
            if encode != 0 {
                let fl: i32;
                fs = if itheta <= (qn >> 1) {
                    itheta + 1
                } else {
                    qn + 1 - itheta
                };
                fl = if itheta <= (qn >> 1) {
                    itheta * (itheta + 1) >> 1
                } else {
                    ft - ((qn + 1 - itheta) * (qn + 2 - itheta) >> 1)
                };
                ec_encode(&mut *ec, fl as u32, (fl + fs) as u32, ft as u32);
            } else {
                /* Triangular pdf */
                let mut fl: i32 = 0;
                let fm: i32;
                fm = ec_decode(&mut *ec, ft as u32) as i32;

                if fm < ((qn >> 1) * ((qn >> 1) + 1) >> 1) {
                    itheta = ((isqrt32(8 * (fm as opus_uint32) + 1) - 1) >> 1) as i32;
                    fs = itheta + 1;
                    fl = itheta * (itheta + 1) >> 1;
                } else {
                    itheta = ((2 * (qn + 1) as opus_uint32
                        - isqrt32(8 * ((ft - fm - 1) as opus_uint32) + 1))
                        >> 1) as i32;
                    fs = qn + 1 - itheta;
                    fl = ft - ((qn + 1 - itheta) * (qn + 2 - itheta) >> 1);
                }

                ec_dec_update(&mut *ec, fl as u32, (fl + fs) as u32, ft as u32);
            }
        }
        celt_assert!(itheta >= 0);
        itheta = celt_udiv((itheta * 16384) as opus_uint32, qn as opus_uint32) as i32;
        if encode != 0 && stereo != 0 {
            if itheta == 0 {
                intensity_stereo(m, alms(X, N), als(Y, N), bandE, i, N);
            } else {
                stereo_split(alms(X, N), alms(Y, N), N);
            }
        }
        /* NOTE: Renormalising X and Y *may* help fixed-point a bit at very high rate.
                 Let's do that at higher complexity */
    } else if stereo != 0 {
        if encode != 0 {
            inv = (itheta > 8192 && ctx.disable_inv == 0) as i32;
            if inv != 0 {
                let mut j: i32 = 0;
                while j < N {
                    *Y.add(j as usize) = -*Y.add(j as usize);
                    j += 1;
                }
            }
            intensity_stereo(m, alms(X, N), als(Y, N), bandE, i, N);
        }
        if *b > 2 << BITRES && ctx.remaining_bits > 2 << BITRES {
            if encode != 0 {
                ec_enc_bit_logp(&mut *ec, inv, 2);
            } else {
                inv = ec_dec_bit_logp(&mut *ec, 2);
            }
        } else {
            inv = 0;
        }
        /* inv flag override to avoid problems with downmixing. */
        if ctx.disable_inv != 0 {
            inv = 0;
        }
        itheta = 0;
    }
    let tell2 = trace::ec_tell_frac(901, &*ec);
    qalloc = tell2 as i32 - tell as i32;
    *b -= qalloc;

    if itheta == 0 {
        imid = 32767;
        iside = 0;
        *fill &= (1 << B) - 1;
        delta = -16384;
    } else if itheta == 16384 {
        imid = 0;
        iside = 32767;
        *fill &= ((1 << B) - 1) << B;
        delta = 16384;
    } else {
        imid = bitexact_cos(itheta as opus_int16) as i32;
        iside = bitexact_cos((16384 - itheta) as opus_int16) as i32;
        /* This is the mid vs side allocation that minimizes squared error
           in that band. */
        delta = FRAC_MUL16((N - 1) << 7, bitexact_log2tan(iside, imid));
    }

    sctx.inv = inv;
    sctx.imid = imid;
    sctx.iside = iside;
    sctx.delta = delta;
    sctx.itheta = itheta;
    sctx.qalloc = qalloc;
}

/// `quant_band_n1()` (`static` in C)
unsafe fn quant_band_n1(
    ctx: &mut band_ctx,
    X: *mut celt_norm,
    Y: *mut celt_norm,
    lowband_out: *mut celt_norm,
) -> u32 {
    let stereo: i32;
    let mut x: *mut celt_norm = X;
    let encode: i32 = ctx.encode;
    let ec: *mut ec_ctx = ctx.ec;

    stereo = (if Y.is_null() { 0 } else { 1 });
    let mut c: i32 = 0;
    loop {
        let mut sign: i32 = 0;
        if ctx.remaining_bits >= 1 << BITRES {
            if encode != 0 {
                sign = (*x < 0.0f32) as i32;
                ec_enc_bits(&mut *ec, sign as u32, 1);
            } else {
                sign = ec_dec_bits(&mut *ec, 1) as i32;
            }
            ctx.remaining_bits -= 1 << BITRES;
        }
        if ctx.resynth != 0 {
            *x = if sign != 0 { -NORM_SCALING } else { NORM_SCALING };
        }
        x = Y;
        c += 1;
        if !(c < 1 + stereo) {
            break;
        }
    }
    if !lowband_out.is_null() {
        *lowband_out = SHR32(*X, 4);
    }
    1
}

/* This function is responsible for encoding and decoding a mono partition.
   It can split the band in two and transmit the energy difference with
   the two half-bands. It can be called recursively so bands can end up being
   split in 8 parts. */
unsafe fn quant_partition(
    ctx: &mut band_ctx,
    X: *mut celt_norm,
    N_in: i32,
    b_in: i32,
    B_in: i32,
    lowband: *mut celt_norm,
    LM_in: i32,
    gain: opus_val32,
    fill_in: i32,
) -> u32 {
    let cache_off: usize;
    let mut q: i32;
    let mut curr_bits: i32;
    let mut cm: u32 = 0;
    let encode: i32 = ctx.encode;
    let m: &CELTMode = ctx.m;
    let i: i32 = ctx.i;
    let spread: i32 = ctx.spread;
    let ec: *mut ec_ctx = ctx.ec;

    let mut N: i32 = N_in;
    let mut b: i32 = b_in;
    let mut B: i32 = B_in;
    let mut LM: i32 = LM_in;
    let mut fill: i32 = fill_in;
    let B0: i32 = B_in;

    /* If we need 1.5 more bit than we can produce, split the band in two. */
    cache_off = m.cache.index[((LM + 1) * m.nbEBands + i) as usize] as usize;
    let cache: &[u8] = &m.cache.bits[cache_off..];
    if LM != -1 && b > cache[cache[0] as usize] as i32 + 12 && N > 2 {
        let mut sbits: i32;
        let qalloc: i32;
        let mut sctx = split_ctx::default();
        let next_lowband2: *mut celt_norm;
        let mut rebalance: opus_int32;

        N >>= 1;
        let Y: *mut celt_norm = X.add(N as usize);
        LM -= 1;
        if B == 1 {
            fill = (fill & 1) | (fill << 1);
        }
        B = (B + 1) >> 1;

        compute_theta(ctx, &mut sctx, X, Y, N, &mut b, B, B0, LM, 0, &mut fill);
        let mut delta: i32 = sctx.delta;
        let itheta: i32 = sctx.itheta;
        qalloc = sctx.qalloc;
        let mid: opus_val32 = (1.0f32 / 32768.0f32) * sctx.imid as opus_val32;
        let side: opus_val32 = (1.0f32 / 32768.0f32) * sctx.iside as opus_val32;

        /* Give more bits to low-energy MDCTs than they would otherwise deserve */
        if B0 > 1 && (itheta & 0x3fff) != 0 {
            if itheta > 8192 {
                /* Rough approximation for pre-echo masking */
                delta -= delta >> (4 - LM);
            } else {
                /* Corresponds to a forward-masking slope of 1.5 dB per 10 ms */
                delta = IMIN(0, delta + (N << BITRES >> (5 - LM)));
            }
        }
        let mut mbits: i32 = IMAX(0, IMIN(b, (b - delta) / 2));
        sbits = b - mbits;
        ctx.remaining_bits -= qalloc;

        if !lowband.is_null() {
            next_lowband2 = lowband.add(N as usize); /* >32-bit split case */
        } else {
            next_lowband2 = core::ptr::null_mut();
        }

        rebalance = ctx.remaining_bits;
        if mbits >= sbits {
            cm = quant_partition(
                ctx,
                X,
                N,
                mbits,
                B,
                lowband,
                LM,
                MULT32_32_Q31(gain, mid),
                fill,
            );
            rebalance = mbits - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 0 {
                sbits += rebalance - (3 << BITRES);
            }
            cm |= quant_partition(
                ctx,
                Y,
                N,
                sbits,
                B,
                next_lowband2,
                LM,
                MULT32_32_Q31(gain, side),
                fill >> B,
            ) << (B0 >> 1);
        } else {
            cm = quant_partition(
                ctx,
                Y,
                N,
                sbits,
                B,
                next_lowband2,
                LM,
                MULT32_32_Q31(gain, side),
                fill >> B,
            ) << (B0 >> 1);
            rebalance = sbits - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 16384 {
                mbits += rebalance - (3 << BITRES);
            }
            cm |= quant_partition(
                ctx,
                X,
                N,
                mbits,
                B,
                lowband,
                LM,
                MULT32_32_Q31(gain, mid),
                fill,
            );
        }
    } else {
        /* This is the basic no-split case */
        q = trace::bits2pulses(m, i, LM, b);
        curr_bits = trace::pulses2bits(m, i, LM, q);
        ctx.remaining_bits -= curr_bits;

        /* Ensures we can never bust the budget */
        while ctx.remaining_bits < 0 && q > 0 {
            ctx.remaining_bits += curr_bits;
            q -= 1;
            curr_bits = trace::pulses2bits(m, i, LM, q);
            ctx.remaining_bits -= curr_bits;
        }

        if q != 0 {
            let K = get_pulses(q);

            /* Finally do the actual quantization */
            if encode != 0 {
                cm = alg_quant(alms(X, N), N, K, spread, B, &mut *ec, gain, ctx.resynth, ctx.arch);
            } else {
                cm = alg_unquant(alms(X, N), N, K, spread, B, &mut *ec, gain);
            }
        } else {
            /* If there's no pulse, fill the band anyway */
            if ctx.resynth != 0 {
                let cm_mask: u32;
                /* B can be as large as 16, so this shift might overflow an int on a
                   16-bit platform; use a long to get defined behavior.*/
                cm_mask = (1u32 << B) - 1;
                fill &= cm_mask as i32;
                if fill == 0 {
                    for j in 0..N as usize {
                        *X.add(j) = 0.0f32;
                    }
                } else {
                    if lowband.is_null() {
                        /* Noise */
                        for j in 0..N {
                            ctx.seed = celt_lcg_rand(ctx.seed);
                            *X.add(j as usize) =
                                SHL32(((ctx.seed as i32) >> 20) as celt_norm, NORM_SHIFT - 14);
                        }
                        cm = cm_mask;
                    } else {
                        /* Folded spectrum */
                        for j in 0..N {
                            let mut tmp: opus_val16;
                            ctx.seed = celt_lcg_rand(ctx.seed);
                            /* About 48 dB below the "normal" folding level */
                            tmp = QCONST16(1.0f32 / 256.0f32, NORM_SHIFT - 4);
                            tmp = if (ctx.seed & 0x8000) != 0 { tmp } else { -tmp };
                            *X.add(j as usize) = *lowband.add(j as usize) + tmp;
                        }
                        cm = fill as u32;
                    }
                    renormalise_vector(alms(X, N), N, gain, ctx.arch);
                }
            }
        }
    }

    cm
}

/* This function is responsible for encoding and decoding a band for the mono case. */
unsafe fn quant_band(
    ctx: &mut band_ctx,
    X: *mut celt_norm,
    N: i32,
    b: i32,
    B_in: i32,
    lowband_in: *mut celt_norm,
    LM: i32,
    lowband_out: *mut celt_norm,
    gain: opus_val32,
    lowband_scratch: *mut celt_norm,
    fill_in: i32,
) -> u32 {
    let N0: i32 = N;
    let mut N_B: i32 = N;
    let mut N_B0: i32 = 0;
    let mut B0: i32 = B_in;
    let mut B: i32 = B_in;
    let mut time_divide: i32 = 0;
    let mut recombine: i32 = 0;
    let longBlocks: i32;
    let mut cm: u32 = 0;
    let k: i32;
    let encode: i32 = ctx.encode;
    let mut tf_change: i32 = ctx.tf_change;
    let mut fill: i32 = fill_in;

    longBlocks = (B0 == 1) as i32;

    N_B = celt_udiv(N_B as opus_uint32, B as opus_uint32) as i32;

    /* Special case for one sample */
    if N == 1 {
        return quant_band_n1(ctx, X, core::ptr::null_mut(), lowband_out);
    }

    let mut lowband: *mut celt_norm = lowband_in;

    if tf_change > 0 {
        recombine = tf_change;
    }
    /* Band recombining to increase frequency resolution */

    if !lowband_scratch.is_null()
        && !lowband.is_null()
        && (recombine != 0 || ((N_B & 1) == 0 && tf_change < 0) || B0 > 1)
    {
        core::ptr::copy_nonoverlapping(lowband, lowband_scratch, N as usize);
        lowband = lowband_scratch;
    }

    for k in 0..recombine {
        static bit_interleave_table: [u8; 16] =
            [0, 1, 1, 1, 2, 3, 3, 3, 2, 3, 3, 3, 2, 3, 3, 3];
        if encode != 0 {
            haar1(alms(X, N), N >> k, 1 << k);
        }
        if !lowband.is_null() {
            haar1(alms(lowband, N), N >> k, 1 << k);
        }
        fill = bit_interleave_table[(fill & 0xF) as usize] as i32
            | (bit_interleave_table[(fill >> 4) as usize] as i32) << 2;
    }
    B >>= recombine;
    N_B <<= recombine;

    /* Increasing the time resolution */
    while (N_B & 1) == 0 && tf_change < 0 {
        if encode != 0 {
            haar1(alms(X, N), N_B, B);
        }
        if !lowband.is_null() {
            haar1(alms(lowband, N), N_B, B);
        }
        fill |= fill << B;
        B <<= 1;
        N_B >>= 1;
        time_divide += 1;
        tf_change += 1;
    }
    B0 = B;
    N_B0 = N_B;

    /* Reorganize the samples in time order instead of frequency order */
    if B0 > 1 {
        if encode != 0 {
            let n0 = N_B >> recombine;
            let stride = B0 << recombine;
            deinterleave_hadamard(alms(X, n0 * stride), n0, stride, longBlocks);
        }
        if !lowband.is_null() {
            let n0 = N_B >> recombine;
            let stride = B0 << recombine;
            deinterleave_hadamard(alms(lowband, n0 * stride), n0, stride, longBlocks);
        }
    }

    cm = quant_partition(ctx, X, N, b, B, lowband, LM, gain, fill);

    /* This code is used by the decoder and by the resynthesis-enabled encoder */
    if ctx.resynth != 0 {
        /* Undo the sample reorganization going from time order to frequency order */
        if B0 > 1 {
            let n0 = N_B >> recombine;
            let stride = B0 << recombine;
            interleave_hadamard(alms(X, n0 * stride), n0, stride, longBlocks);
        }

        /* Undo time-freq changes that we did earlier */
        N_B = N_B0;
        B = B0;
        for _k in 0..time_divide {
            B >>= 1;
            N_B <<= 1;
            cm |= cm >> B;
            haar1(alms(X, N), N_B, B);
        }

        for k in 0..recombine {
            static bit_deinterleave_table: [u8; 16] = [
                0x00, 0x03, 0x0C, 0x0F, 0x30, 0x33, 0x3C, 0x3F, 0xC0, 0xC3, 0xCC, 0xCF, 0xF0,
                0xF3, 0xFC, 0xFF,
            ];
            cm = bit_deinterleave_table[cm as usize] as u32;
            haar1(alms(X, N), N0 >> k, 1 << k);
        }
        B <<= recombine;

        /* Scale output for later folding */
        if !lowband_out.is_null() {
            let n: opus_val16;
            n = celt_sqrt(SHL32(EXTEND32(N0 as opus_val32), 22));
            for j in 0..N0 {
                *lowband_out.add(j as usize) = MULT16_32_Q15(n, *X.add(j as usize));
            }
        }
        cm &= (1u32 << B) - 1;
    }
    let _ = k;
    cm
}

/* This function is responsible for encoding and decoding a band for the stereo case. */
unsafe fn quant_band_stereo(
    ctx: &mut band_ctx,
    X: *mut celt_norm,
    Y: *mut celt_norm,
    N: i32,
    b_in: i32,
    B: i32,
    lowband: *mut celt_norm,
    LM: i32,
    lowband_out: *mut celt_norm,
    lowband_scratch: *mut celt_norm,
    fill_in: i32,
) -> u32 {
    let mut cm: u32 = 0;
    let mut sctx = split_ctx::default();
    let orig_fill: i32;
    let ec: *mut ec_ctx = ctx.ec;
    let encode: i32 = ctx.encode;
    let mut b: i32 = b_in;
    let mut fill: i32 = fill_in;

    /* Special case for one sample */
    if N == 1 {
        return quant_band_n1(ctx, X, Y, lowband_out);
    }

    orig_fill = fill;

    if encode != 0 {
        if ctx.bandE[ctx.i as usize] < MIN_STEREO_ENERGY
            || ctx.bandE[(ctx.m.nbEBands + ctx.i) as usize] < MIN_STEREO_ENERGY
        {
            if ctx.bandE[ctx.i as usize] > ctx.bandE[(ctx.m.nbEBands + ctx.i) as usize] {
                core::ptr::copy_nonoverlapping(X, Y, N as usize);
            } else {
                core::ptr::copy_nonoverlapping(Y, X, N as usize);
            }
        }
    }
    compute_theta(ctx, &mut sctx, X, Y, N, &mut b, B, B, LM, 1, &mut fill);
    let inv: i32 = sctx.inv;
    let itheta: i32 = sctx.itheta;
    let delta: i32 = sctx.delta;
    let qalloc: i32 = sctx.qalloc;
    let mid: opus_val32 = (1.0f32 / 32768.0f32) * sctx.imid as opus_val32;
    let side: opus_val32 = (1.0f32 / 32768.0f32) * sctx.iside as opus_val32;

    /* This is a special case for N=2 that only works for stereo and takes
       advantage of the fact that mid and side are orthogonal to encode
       the side with just one bit. */
    if N == 2 {
        let c: i32;
        let mut sign: i32 = 0;
        let x2: *mut celt_norm;
        let y2: *mut celt_norm;
        let mut mbits: i32 = b;
        let mut sbits: i32 = 0;
        /* Only need one bit for the side. */
        if itheta != 0 && itheta != 16384 {
            sbits = 1 << BITRES;
        }
        mbits -= sbits;
        c = (itheta > 8192) as i32;
        ctx.remaining_bits -= qalloc + sbits;

        x2 = if c != 0 { Y } else { X };
        y2 = if c != 0 { X } else { Y };
        if sbits != 0 {
            if encode != 0 {
                /* Here we only need to encode a sign for the side. */
                sign = (MULT32_32_Q31(*x2, *y2.add(1)) - MULT32_32_Q31(*x2.add(1), *y2)
                    < 0.0f32) as i32;
                ec_enc_bits(&mut *ec, sign as u32, 1);
            } else {
                sign = ec_dec_bits(&mut *ec, 1) as i32;
            }
        }
        sign = 1 - 2 * sign;
        /* We use orig_fill here because we want to fold the side, but if
           itheta==16384, we'll have cleared the low bits of fill. */
        cm = quant_band(
            ctx,
            x2,
            N,
            mbits,
            B,
            lowband,
            LM,
            lowband_out,
            Q31ONE,
            lowband_scratch,
            orig_fill,
        );
        /* We don't split N=2 bands, so cm is either 1 or 0 (for a fold-collapse),
           and there's no need to worry about mixing with the other channel. */
        *y2.add(0) = -(sign as opus_val32) * *x2.add(1);
        *y2.add(1) = (sign as opus_val32) * *x2.add(0);
        if ctx.resynth != 0 {
            let tmp: celt_norm;
            *X = MULT32_32_Q31(mid, *X);
            *X.add(1) = MULT32_32_Q31(mid, *X.add(1));
            *Y = MULT32_32_Q31(side, *Y);
            *Y.add(1) = MULT32_32_Q31(side, *Y.add(1));
            tmp = *X;
            *X = SUB32(tmp, *Y);
            *Y = ADD32(tmp, *Y);
            let tmp = *X.add(1);
            *X.add(1) = SUB32(tmp, *Y.add(1));
            *Y.add(1) = ADD32(tmp, *Y.add(1));
        }
    } else {
        /* "Normal" split code */
        let mut rebalance: opus_int32;

        let mut mbits: i32 = IMAX(0, IMIN(b, (b - delta) / 2));
        let mut sbits: i32 = b - mbits;
        ctx.remaining_bits -= qalloc;

        rebalance = ctx.remaining_bits;
        if mbits >= sbits {
            /* In stereo mode, we do not apply a scaling to the mid because we need the normalized
               mid for folding later. */
            cm = quant_band(
                ctx,
                X,
                N,
                mbits,
                B,
                lowband,
                LM,
                lowband_out,
                Q31ONE,
                lowband_scratch,
                fill,
            );
            rebalance = mbits - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 0 {
                sbits += rebalance - (3 << BITRES);
            }
            /* For a stereo split, the high bits of fill are always zero, so no
               folding will be done to the side. */
            cm |= quant_band(
                ctx,
                Y,
                N,
                sbits,
                B,
                core::ptr::null_mut(),
                LM,
                core::ptr::null_mut(),
                side,
                core::ptr::null_mut(),
                fill >> B,
            );
        } else {
            /* For a stereo split, the high bits of fill are always zero, so no
               folding will be done to the side. */
            cm = quant_band(
                ctx,
                Y,
                N,
                sbits,
                B,
                core::ptr::null_mut(),
                LM,
                core::ptr::null_mut(),
                side,
                core::ptr::null_mut(),
                fill >> B,
            );
            rebalance = sbits - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 16384 {
                mbits += rebalance - (3 << BITRES);
            }
            /* In stereo mode, we do not apply a scaling to the mid because we need the normalized
               mid for folding later. */
            cm |= quant_band(
                ctx,
                X,
                N,
                mbits,
                B,
                lowband,
                LM,
                lowband_out,
                Q31ONE,
                lowband_scratch,
                fill,
            );
        }
    }

    /* This code is used by the decoder and by the resynthesis-enabled encoder */
    if ctx.resynth != 0 {
        if N != 2 {
            stereo_merge(alms(X, N), alms(Y, N), mid, N, ctx.arch);
        }
        if inv != 0 {
            for j in 0..N as usize {
                *Y.add(j) = -*Y.add(j);
            }
        }
    }
    cm
}

/// `special_hybrid_folding()` (`static` in C, `#ifndef DISABLE_UPDATE_DRAFT`)
pub unsafe fn special_hybrid_folding(
    m: &CELTMode,
    norm: *mut celt_norm,
    norm2: *mut celt_norm,
    start: i32,
    M: i32,
    dual_stereo: i32,
) {
    let n1: i32;
    let n2: i32;
    let eBands = &m.eBands;
    n1 = M * (eBands[(start + 1) as usize] as i32 - eBands[start as usize] as i32);
    n2 = M * (eBands[(start + 2) as usize] as i32 - eBands[(start + 1) as usize] as i32);
    /* Duplicate enough of the first band folding data to be able to fold the second band.
       Copies no data for CELT-only mode. */
    for j in 0..(n2 - n1) {
        *norm.add((n1 + j) as usize) = *norm.add((2 * n1 - n2 + j) as usize);
        if dual_stereo != 0 {
            *norm2.add((n1 + j) as usize) = *norm2.add((2 * n1 - n2 + j) as usize);
        }
    }
}

/// Restore every `ec_ctx` field except the buffer contents (`*ec = ec_save` in C
/// copies the pointer, not the bytes; `bytes_save` handles the bytes).
#[inline]
fn ec_restore_scalars(dst: &mut ec_ctx, src: &ec_ctx) {
    dst.storage = src.storage;
    dst.end_offs = src.end_offs;
    dst.end_window = src.end_window;
    dst.nend_bits = src.nend_bits;
    dst.nbits_total = src.nbits_total;
    dst.offs = src.offs;
    dst.rng = src.rng;
    dst.val = src.val;
    dst.ext = src.ext;
    dst.rem = src.rem;
    dst.error = src.error;
}

/// `quant_all_bands()`
///
/// * `X_` — the interleaved normalised MDCTs (the second channel, when
///   `C == 2`, lives in `Y_`; upstream passes `X_+N`).
/// * `Y_` — `None` for mono.
/// * `seed` — in/out (`opus_uint32 *seed` upstream).
#[allow(clippy::too_many_arguments)]
pub fn quant_all_bands(
    encode: i32,
    m: &CELTMode,
    start: i32,
    end: i32,
    X_: &mut [celt_norm],
    mut Y_: Option<&mut [celt_norm]>,
    collapse_masks: &mut [u8],
    bandE: &[celt_ener],
    pulses: &[i32],
    shortBlocks: i32,
    spread: i32,
    dual_stereo_in: i32,
    intensity: i32,
    tf_res: &[i32],
    total_bits: opus_int32,
    balance_in: opus_int32,
    ec: &mut ec_ctx,
    LM: i32,
    codedBands: i32,
    seed: &mut opus_uint32,
    complexity: i32,
    arch: i32,
    disable_inv: i32,
) {
    let mut remaining_bits: opus_int32;
    let eBands = &m.eBands;
    let C: i32 = if Y_.is_some() { 2 } else { 1 };
    let M: i32;
    let mut B: i32;
    let mut lowband_offset: i32;
    let mut update_lowband: i32 = 1;
    let norm_offset: i32;
    let mut dual_stereo = dual_stereo_in;

    M = 1 << LM;
    B = if shortBlocks != 0 { M } else { 1 };
    norm_offset = M * eBands[start as usize] as i32;

    let theta_rdo: i32 =
        (encode != 0 && Y_.is_some() && dual_stereo_in == 0 && complexity >= 8) as i32;
    let resynth: i32 = if encode == 0 { 1 } else { theta_rdo };

    /* The internal `norm`/`norm2` scratch: `norm2` starts at
       `M*eBands[nbEBands-1] - norm_offset` inside the same allocation, exactly
       like `norm2 = norm + M*eBands[nbEBands-1]-norm_offset`. */
    let norm_span: i32 = M * eBands[m.nbEBands as usize - 1] as i32 - norm_offset;
    let norm_alloc: i32 = C * norm_span;
    /* `VARDECL(celt_norm, _norm); ALLOC(_norm, C*norm_span, celt_norm);`:
       `norm_span <= M*shortMdctSize == N <= MAX_CELT_FRAME`, so this is the
       first half of that C allocation and `norm2` the second, exactly as the
       C pointer arithmetic `norm2 = norm + norm_span`. */
    let mut norm_storage = [0.0f32; MAX_CHANNELS * MAX_CELT_FRAME];
    let _norm = &mut norm_storage[..norm_alloc as usize];
    let norm: *mut celt_norm = _norm.as_mut_ptr();
    let norm2: *mut celt_norm = unsafe { norm.add(norm_span as usize) };

    /* `resynth_alloc` is only nonzero when the encoder resynthesises.  Upstream
       points `lowband_scratch` into the caller's `X_` for the decoder; that
       region is always overwritten before it is read (see the module comment),
       so an owned buffer is observationally identical and keeps every mode
       panic-free.  The size is the widest band of the mode, which is at least
       upstream's `M*(eBands[nbEBands]-eBands[nbEBands-1])`. */
    let max_diff: i32 = {
        let mut md = 0;
        let mut j = 0;
        while j < m.nbEBands {
            let d = eBands[j as usize + 1] as i32 - eBands[j as usize] as i32;
            if d > md {
                md = d;
            }
            j += 1;
        }
        md
    };
    let resynth_alloc: i32 = M * max_diff;
    /* `VARDECL(celt_norm, _lowband_scratch/X_save/Y_save/X_save2/Y_save2/
       norm_save2); ALLOC(..., resynth_alloc, ...)` — `resynth_alloc <= N`
       because every band is at most the last one and
       `eBands[nbEBands] <= shortMdctSize`, so all six are stack arrays. */
    let mut lowband_scratch_storage = [0.0f32; MAX_CELT_FRAME];
    let _lowband_scratch = &mut lowband_scratch_storage[..resynth_alloc as usize];
    let mut x_save_storage = [0.0f32; MAX_CELT_FRAME];
    let _x_save = &mut x_save_storage[..resynth_alloc as usize];
    let mut y_save_storage = [0.0f32; MAX_CELT_FRAME];
    let _y_save = &mut y_save_storage[..resynth_alloc as usize];
    let mut x_save2_storage = [0.0f32; MAX_CELT_FRAME];
    let _x_save2 = &mut x_save2_storage[..resynth_alloc as usize];
    let mut y_save2_storage = [0.0f32; MAX_CELT_FRAME];
    let _y_save2 = &mut y_save2_storage[..resynth_alloc as usize];
    let mut norm_save2_storage = [0.0f32; MAX_CELT_FRAME];
    let _norm_save2 = &mut norm_save2_storage[..resynth_alloc as usize];
    let x_save: *mut celt_norm = _x_save.as_mut_ptr();
    let y_save: *mut celt_norm = _y_save.as_mut_ptr();
    let x_save2: *mut celt_norm = _x_save2.as_mut_ptr();
    let y_save2: *mut celt_norm = _y_save2.as_mut_ptr();
    let norm_save2: *mut celt_norm = _norm_save2.as_mut_ptr();
    let ecp: *mut ec_ctx = ec;
    let x_base: *mut celt_norm = X_.as_mut_ptr();
    let y_base: *mut celt_norm = match Y_ {
        Some(ref mut y) => y.as_mut_ptr(),
        None => core::ptr::null_mut(),
    };
    /* `if (encode && resynth) lowband_scratch = _lowband_scratch; else
        lowband_scratch = X_+M*eBands[effEBands-1];` — the decoder (and the
       non-resynthesising encoder) really does use the tail of the caller's
       `X_` array as scratch, which is observable in `X` beyond the coded
       bands; reproduce it.  The fallback keeps a hypothetical mode whose
       widest band is not the last from writing past the caller's buffer
       (upstream would be out of bounds there). */
    let scratch_base: *mut celt_norm = if encode != 0 && resynth != 0 {
        _lowband_scratch.as_mut_ptr()
    } else {
        let s = (M * eBands[(m.effEBands - 1) as usize] as i32) as usize;
        if s + resynth_alloc as usize <= X_.len() {
            unsafe { x_base.add(s) }
        } else {
            _lowband_scratch.as_mut_ptr()
        }
    };

    let mut ctx: band_ctx = band_ctx {
        encode,
        resynth,
        m,
        i: 0,
        intensity,
        spread,
        tf_change: 0,
        ec: ecp,
        remaining_bits: 0,
        bandE,
        seed: *seed,
        arch,
        theta_round: 0,
        disable_inv,
        avoid_split_noise: (B > 1) as i32,
    };

    /* `ALLOC(bytes_save, theta_rdo ? 1275 : ALLOC_NONE, unsigned char);` —
       only used when `theta_rdo`, which implies `encode` and therefore
       `ec.storage <= packet_size_cap == 1275`. */
    let mut bytes_save = [0u8; 1275];

    lowband_offset = 0;
    let mut balance = balance_in;
    let mut i = start;
    while i < end {
        let tell: opus_uint32;
        let b: i32;
        let N: i32;
        let curr_balance: opus_int32;
        let mut effective_lowband: i32 = -1;
        let mut X: *mut celt_norm;
        let mut Y: *mut celt_norm;
        let mut tf_change: i32 = 0;
        let mut x_cm: u32;
        let mut y_cm: u32;
        let last: i32;
        let mut lowband_scratch: *mut celt_norm = scratch_base;

        ctx.i = i;
        last = (i == end - 1) as i32;

        X = unsafe { x_base.add((M * eBands[i as usize] as i32) as usize) };
        if !y_base.is_null() {
            Y = unsafe { y_base.add((M * eBands[i as usize] as i32) as usize) };
        } else {
            Y = core::ptr::null_mut();
        }
        N = M * eBands[i as usize + 1] as i32 - M * eBands[i as usize] as i32;
        celt_assert!(N > 0);
        tell = trace::ec_tell_frac(1706, unsafe { &*ecp });

        /* Compute how many bits we want to allocate to this band */
        if i != start {
            balance -= tell as opus_int32;
        }
        remaining_bits = total_bits - tell as opus_int32 - 1;
        ctx.remaining_bits = remaining_bits;
        if i <= codedBands - 1 {
            curr_balance = celt_sudiv(balance, IMIN(3, codedBands - i));
            b = IMAX(
                0,
                IMIN(
                    16383,
                    IMIN(remaining_bits + 1, pulses[i as usize] + curr_balance),
                ),
            );
        } else {
            b = 0;
        }

        if resynth != 0
            && ((M * eBands[i as usize] as i32 - N >= M * eBands[start as usize] as i32)
                || i == start + 1)
            && (update_lowband != 0 || lowband_offset == 0)
        {
            lowband_offset = i;
        }
        if i == start + 1 {
            unsafe { special_hybrid_folding(m, norm, norm2, start, M, dual_stereo) };
        }

        tf_change = tf_res[i as usize];
        ctx.tf_change = tf_change;
        if i >= m.effEBands {
            X = norm;
            if !y_base.is_null() {
                Y = norm;
            }
            lowband_scratch = core::ptr::null_mut();
        }
        if last != 0 && theta_rdo == 0 {
            lowband_scratch = core::ptr::null_mut();
        }

        /* Get a conservative estimate of the collapse_mask's for the bands we're
           going to be folding from. */
        if lowband_offset != 0 && (spread != SPREAD_AGGRESSIVE || B > 1 || tf_change < 0) {
            /* This ensures we never repeat spectral content within one band */
            effective_lowband = IMAX(
                0,
                M * eBands[lowband_offset as usize] as i32 - norm_offset - N,
            );
            let mut fs = lowband_offset;
            loop {
                fs -= 1;
                if M * eBands[fs as usize] as i32 <= effective_lowband + norm_offset {
                    break;
                }
            }
            let fold_start = fs;
            let mut fe = lowband_offset - 1;
            loop {
                fe += 1;
                if !(fe < i
                    && M * (eBands[fe as usize] as i32) < effective_lowband + norm_offset + N)
                {
                    break;
                }
            }
            let fold_end = fe;
            x_cm = 0;
            y_cm = 0;
            let mut fold_i = fold_start;
            loop {
                x_cm |= collapse_masks[(fold_i * C + 0) as usize] as u32;
                y_cm |= collapse_masks[(fold_i * C + C - 1) as usize] as u32;
                fold_i += 1;
                if !(fold_i < fold_end) {
                    break;
                }
            }
        } else {
            /* Otherwise, we'll be using the LCG to fold, so all blocks will (almost
               always) be non-zero. */
            x_cm = (1u32 << B) - 1;
            y_cm = x_cm;
        }

        if dual_stereo != 0 && i == intensity {
            /* Switch off dual stereo to do intensity. */
            dual_stereo = 0;
            if resynth != 0 {
                for j in 0..(M * eBands[i as usize] as i32 - norm_offset) {
                    unsafe {
                        *norm.add(j as usize) =
                            HALF32(*norm.add(j as usize) + *norm2.add(j as usize));
                    }
                }
            }
        }
        let lb: *mut celt_norm = if effective_lowband != -1 {
            unsafe { norm.add(effective_lowband as usize) }
        } else {
            core::ptr::null_mut()
        };
        let lo: *mut celt_norm = if last == 0 {
            unsafe { norm.add((M * eBands[i as usize] as i32 - norm_offset) as usize) }
        } else {
            core::ptr::null_mut()
        };
        if dual_stereo != 0 {
            let ylb: *mut celt_norm = if effective_lowband != -1 {
                unsafe { norm2.add(effective_lowband as usize) }
            } else {
                core::ptr::null_mut()
            };
            let ylo: *mut celt_norm = if last == 0 {
                unsafe { norm2.add((M * eBands[i as usize] as i32 - norm_offset) as usize) }
            } else {
                core::ptr::null_mut()
            };
            x_cm = unsafe {
                quant_band(
                    &mut ctx,
                    X,
                    N,
                    b / 2,
                    B,
                    lb,
                    LM,
                    lo,
                    Q31ONE,
                    lowband_scratch,
                    x_cm as i32,
                )
            };
            y_cm = unsafe {
                quant_band(
                    &mut ctx,
                    Y,
                    N,
                    b / 2,
                    B,
                    ylb,
                    LM,
                    ylo,
                    Q31ONE,
                    lowband_scratch,
                    y_cm as i32,
                )
            };
        } else if !Y.is_null() {
            if theta_rdo != 0 && i < intensity {
                let mut w: [opus_val16; 2] = [0.0f32; 2];
                let cm0: u32;
                let cm2: u32;
                let dist0: opus_val32;
                let dist1: opus_val32;
                let ec_save: ec_ctx;
                let ec_save2: ec_ctx;
                let ctx_save: band_ctx;
                let ctx_save2: band_ctx;
                let nstart_bytes: usize;
                let nend_bytes: usize;
                let save_bytes: usize;
                compute_channel_weights(
                    bandE[i as usize],
                    bandE[(i + m.nbEBands) as usize],
                    &mut w,
                );
                /* Make a copy. */
                cm0 = x_cm | y_cm;
                ec_save = unsafe { (*ecp).clone() };
                ctx_save = ctx;
                unsafe {
                    core::ptr::copy_nonoverlapping(X, x_save, N as usize);
                    core::ptr::copy_nonoverlapping(Y, y_save, N as usize);
                }
                /* Encode and round down. */
                ctx.theta_round = -1;
                x_cm = unsafe {
                    quant_band_stereo(
                        &mut ctx,
                        X,
                        Y,
                        N,
                        b,
                        B,
                        lb,
                        LM,
                        lo,
                        lowband_scratch,
                        cm0 as i32,
                    )
                };
                dist0 = MULT16_32_Q15(w[0], unsafe {
                    celt_inner_prod(als(x_save, N), als(X, N), N)
                }) + MULT16_32_Q15(w[1], unsafe {
                    celt_inner_prod(als(y_save, N), als(Y, N), N)
                });

                /* Save first result. */
                cm2 = x_cm;
                ec_save2 = unsafe { (*ecp).clone() };
                ctx_save2 = ctx;
                unsafe {
                    core::ptr::copy_nonoverlapping(X, x_save2, N as usize);
                    core::ptr::copy_nonoverlapping(Y, y_save2, N as usize);
                }
                if last == 0 {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            norm.add((M * eBands[i as usize] as i32 - norm_offset) as usize),
                            norm_save2,
                            N as usize,
                        );
                    }
                }
                nstart_bytes = ec_save.offs as usize;
                nend_bytes = ec_save.storage as usize;
                save_bytes = nend_bytes - nstart_bytes;
                let ec_buf: &[u8] = unsafe { &(*ecp).buf };
                bytes_save[..save_bytes].copy_from_slice(&ec_buf[nstart_bytes..nend_bytes]);
                /* Restore */
                ec_restore_scalars(unsafe { &mut *ecp }, &ec_save);
                ctx = ctx_save;
                unsafe {
                    core::ptr::copy_nonoverlapping(x_save, X, N as usize);
                    core::ptr::copy_nonoverlapping(y_save, Y, N as usize);
                }
                if i == start + 1 {
                    unsafe { special_hybrid_folding(m, norm, norm2, start, M, dual_stereo) };
                }
                /* Encode and round up. */
                ctx.theta_round = 1;
                x_cm = unsafe {
                    quant_band_stereo(
                        &mut ctx,
                        X,
                        Y,
                        N,
                        b,
                        B,
                        lb,
                        LM,
                        lo,
                        lowband_scratch,
                        cm0 as i32,
                    )
                };
                dist1 = MULT16_32_Q15(w[0], unsafe {
                    celt_inner_prod(als(x_save, N), als(X, N), N)
                }) + MULT16_32_Q15(w[1], unsafe {
                    celt_inner_prod(als(y_save, N), als(Y, N), N)
                });
                if dist0 >= dist1 {
                    x_cm = cm2;
                    ec_restore_scalars(unsafe { &mut *ecp }, &ec_save2);
                    ctx = ctx_save2;
                    unsafe {
                        core::ptr::copy_nonoverlapping(x_save2, X, N as usize);
                        core::ptr::copy_nonoverlapping(y_save2, Y, N as usize);
                    }
                    if last == 0 {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                norm_save2,
                                norm.add((M * eBands[i as usize] as i32 - norm_offset) as usize),
                                N as usize,
                            );
                        }
                    }
                    let ec_buf: &mut [u8] = unsafe { &mut (*ecp).buf };
                    ec_buf[nstart_bytes..nend_bytes].copy_from_slice(&bytes_save[..save_bytes]);
                }
            } else {
                ctx.theta_round = 0;
                x_cm = unsafe {
                    quant_band_stereo(
                        &mut ctx,
                        X,
                        Y,
                        N,
                        b,
                        B,
                        lb,
                        LM,
                        lo,
                        lowband_scratch,
                        (x_cm | y_cm) as i32,
                    )
                };
            }
            y_cm = x_cm;
        } else {
            x_cm = unsafe {
                quant_band(
                    &mut ctx,
                    X,
                    N,
                    b,
                    B,
                    lb,
                    LM,
                    lo,
                    Q31ONE,
                    lowband_scratch,
                    (x_cm | y_cm) as i32,
                )
            };
            y_cm = x_cm;
        }
        collapse_masks[(i * C + 0) as usize] = x_cm as u8;
        collapse_masks[(i * C + C - 1) as usize] = y_cm as u8;
        balance += pulses[i as usize] + tell as opus_int32;

        /* Update the folding position only as long as we have 1 bit/sample depth. */
        update_lowband = (b > (N << BITRES)) as i32;
        /* We only need to avoid noise on a split for the first band. After that, we
           have folding. */
        ctx.avoid_split_noise = 0;
        i += 1;
    }
    *seed = ctx.seed;
}

// ---------------------------------------------------------------------------
// Probe-only instrumentation.
//
// `compat/c/probe_bands.c` includes `celt/bands.c` and hooks the four call
// sites below with `#define`s (`ec_tell_frac`, `bits2pulses`, `pulses2bits`,
// `stereo_itheta`).  These wrappers are byte-for-byte pass-throughs; they only
// *record* when `examples/probe_bands.rs` turns the switch on, so the
// arithmetic is untouched.
// ---------------------------------------------------------------------------

/// Probe-only tracing of the band-loop intermediates.
#[doc(hidden)]
pub mod trace {
    use super::super::arch::celt_norm;
    use super::super::entcode::{ec_ctx, ec_tell_frac as rate_ec_tell_frac};
    use super::super::modes::CELTMode;
    use super::super::rate::{bits2pulses as rate_bits2pulses, pulses2bits as rate_pulses2bits};
    use super::super::vq::stereo_itheta as vq_stereo_itheta;
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::cell::RefCell;

    /// `(tag, a, b, c)`, in call order.
    pub type Event = (i32, i64, i64, i64);

    static ON: AtomicBool = AtomicBool::new(false);

    thread_local! {
        static EVENTS: RefCell<Vec<Event>> = RefCell::new(Vec::new());
    }

    /// Enable/disable recording; also clears the event list.
    pub fn set(on: bool) {
        ON.store(on, Ordering::Relaxed);
        EVENTS.with(|e| e.borrow_mut().clear());
    }

    /// Take the recorded events.
    pub fn take() -> Vec<Event> {
        EVENTS.with(|e| core::mem::take(&mut *e.borrow_mut()))
    }

    /// True when recording is on (lets the probe skip the capture entirely).
    pub fn enabled() -> bool {
        ON.load(Ordering::Relaxed)
    }

    #[inline(always)]
    fn push(tag: i32, a: i64, b: i64, c: i64) {
        if ON.load(Ordering::Relaxed) {
            EVENTS.with(|e| e.borrow_mut().push((tag, a, b, c)));
        }
    }

    /// `ec_tell_frac()` at one of the three call sites in `bands.c`
    /// (tags 744, 901, 1706).
    #[inline(always)]
    pub fn ec_tell_frac(tag: i32, ec: &ec_ctx) -> u32 {
        let v = rate_ec_tell_frac(ec);
        push(tag, v as i64, 0, 0);
        v
    }

    /// `bits2pulses()` (`bands.c:1094`)
    #[inline(always)]
    pub fn bits2pulses(m: &CELTMode, band: i32, LM: i32, bits: i32) -> i32 {
        let q = rate_bits2pulses(m, band, LM, bits);
        push(1094, LM as i64, bits as i64, q as i64);
        q
    }

    /// `pulses2bits()` (`bands.c:1095`, also the `while` loop below it)
    #[inline(always)]
    pub fn pulses2bits(m: &CELTMode, band: i32, LM: i32, pulses: i32) -> i32 {
        let v = rate_pulses2bits(m, band, LM, pulses);
        push(1095, LM as i64, pulses as i64, v as i64);
        v
    }

    /// `stereo_itheta()` (`bands.c:741`)
    #[inline(always)]
    pub fn stereo_itheta(X: &[celt_norm], Y: &[celt_norm], stereo: i32, N: i32, arch: i32) -> i32 {
        let v = vq_stereo_itheta(X, Y, stereo, N, arch);
        push(741, v as i64, 0, 0);
        v
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::modes::opus_custom_mode_create;
    use super::*;

    fn mode() -> Box<CELTMode> {
        let mut err = 0;
        opus_custom_mode_create(48000, 960, &mut err).unwrap()
    }

    #[test]
    fn haar1_known_values() {
        /* First stage of a 4-point Haar: [1,0,0,0] -> [c,c,0,0]. */
        let c: f32 = QCONST32(0.70710678f32, 31);
        let mut x = [1.0f32, 0.0, 0.0, 0.0];
        haar1(&mut x, 4, 1);
        assert_eq!(x[0], c * 1.0f32);
        assert_eq!(x[1], c * 1.0f32);
        assert_eq!(x[2], 0.0);
        assert_eq!(x[3], 0.0);
        let mut y = x;
        haar1(&mut y, 4, 1);
        assert_eq!(y[0], MULT32_32_Q31(c, c) + MULT32_32_Q31(c, c));
        assert_eq!(y[1], 0.0);
    }

    #[test]
    fn haar1_stride() {
        let c: f32 = QCONST32(0.70710678f32, 31);
        /* N0=4, stride=2 -> 8 samples; the index is `stride*2*j + i`, so the
           pairs are (0,2), (4,6) for i=0 and (1,3), (5,7) for i=1. */
        let mut x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        haar1(&mut x, 4, 2);
        assert_eq!(x[0], c * 1.0f32 + c * 3.0f32);
        assert_eq!(x[2], c * 1.0f32 - c * 3.0f32);
        assert_eq!(x[4], c * 5.0f32 + c * 7.0f32);
        assert_eq!(x[6], c * 5.0f32 - c * 7.0f32);
        assert_eq!(x[1], c * 2.0f32 + c * 4.0f32);
        assert_eq!(x[3], c * 2.0f32 - c * 4.0f32);
        assert_eq!(x[5], c * 6.0f32 + c * 8.0f32);
        assert_eq!(x[7], c * 6.0f32 - c * 8.0f32);
    }

    #[test]
    fn interleave_deinterleave_roundtrip() {
        for &stride in &[2i32, 4, 8, 16] {
            for &had in &[0i32, 1] {
                let n = 32 * stride;
                let mut orig = vec![0.0f32; n as usize];
                let mut st: u32 = 1;
                for v in orig.iter_mut() {
                    st = st.wrapping_mul(1103515245).wrapping_add(12345);
                    *v = ((st >> 8) as i32 - 8388608) as f32 / 8388608.0;
                }
                let mut x = orig.clone();
                interleave_hadamard(&mut x, 32, stride, had);
                assert_ne!(x, orig, "interleave must permute");
                deinterleave_hadamard(&mut x, 32, stride, had);
                assert_eq!(x, orig, "hadamard={} stride={}", had, stride);
            }
        }
    }

    #[test]
    fn hysteresis_decision_matches_c() {
        let thresholds: [f32; 5] = [0.3, 0.6, 0.9, 1.2, 1.5];
        let hysteresis: [f32; 5] = [0.05, 0.05, 0.05, 0.05, 0.05];
        assert_eq!(hysteresis_decision(0.1, &thresholds, &hysteresis, 5, 0), 0);
        assert_eq!(hysteresis_decision(0.7, &thresholds, &hysteresis, 5, 0), 2);
        /* i==1 < prev==2 and 0.5 <= 0.6-0.05 -> move down */
        assert_eq!(hysteresis_decision(0.5, &thresholds, &hysteresis, 5, 2), 1);
        /* i==1 < prev==2 and 0.58 > 0.6-0.05 -> keep prev */
        assert_eq!(hysteresis_decision(0.58, &thresholds, &hysteresis, 5, 2), 2);
        /* i==3 > prev==2 but 0.94 < 0.9+0.05 -> keep prev */
        assert_eq!(hysteresis_decision(0.94, &thresholds, &hysteresis, 5, 2), 2);
        /* i==3 > prev==2 and 0.96 >= 0.95 -> move */
        assert_eq!(hysteresis_decision(0.96, &thresholds, &hysteresis, 5, 2), 3);
        assert_eq!(hysteresis_decision(2.0, &thresholds, &hysteresis, 5, 4), 5);
    }

    #[test]
    fn band_energies_and_normalise() {
        let m = mode();
        let n = m.shortMdctSize;
        let mut x = vec![0.0f32; n as usize];
        let mut st: u32 = 7;
        for (k, v) in x.iter_mut().enumerate() {
            st = st.wrapping_mul(1103515245).wrapping_add(12345);
            *v = ((st >> 8) as i32 - 8388608) as f32 / 8388608.0 * (1.0 + (k % 3) as f32);
        }
        let mut e = vec![0.0f32; m.nbEBands as usize];
        compute_band_energies(&m, &x, &mut e, m.nbEBands, 1, 0, 4);
        for i in 0..m.nbEBands as usize {
            assert!(e[i] > 0.0 && e[i].is_finite());
        }
        let mut xn = vec![0.0f32; n as usize];
        normalise_bands(&m, &x, &mut xn, &e, m.nbEBands, 1, 1);
        /* Each normalised band has unit energy (within float rounding). */
        let mut e2 = vec![0.0f32; m.nbEBands as usize];
        compute_band_energies(&m, &xn, &mut e2, m.nbEBands, 1, 0, 4);
        for i in 0..m.nbEBands as usize {
            if e[i] > 1e-6 {
                assert!((e2[i] - 1.0).abs() < 0.05, "band {}: {}", i, e2[i]);
            }
        }
    }

    #[test]
    fn denormalise_clears_tail() {
        let m = mode();
        let n = m.shortMdctSize;
        let x = vec![1.0f32; n as usize];
        let log_e = vec![0.0f32; m.nbEBands as usize];
        let mut freq = vec![9.0f32; n as usize];
        denormalise_bands(&m, &x, &mut freq, &log_e, 0, 4, 1, 1, 0);
        let bound = m.eBands[4] as usize;
        for k in bound..n as usize {
            assert_eq!(freq[k], 0.0);
        }
        for k in 0..bound {
            assert!(freq[k] > 0.0);
        }
    }

    #[test]
    fn compute_qn_bounds() {
        /* Property: qn is 1 or an even value <= 256. */
        for n in 2..64i32 {
            for b in 0..400i32 {
                let qn = compute_qn(n, b, 4, 24, 0);
                assert!(qn == 1 || (qn & 1) == 0);
                assert!(qn <= 256 && qn >= 1);
            }
        }
    }

    #[test]
    fn stereo_split_known_values() {
        let mut x = [0.5f32, -0.25];
        let mut y = [0.125f32, 0.75];
        let c: f32 = QCONST32(0.70710678f32, 31);
        let x0 = x[0];
        let y0 = y[0];
        stereo_split(&mut x, &mut y, 2);
        assert_eq!(x[0], c * x0 + c * y0);
        assert_eq!(y[0], c * y0 - c * x0);
    }

    #[test]
    fn anti_collapse_fills_collapsed_blocks() {
        let m = mode();
        let lm = 1;
        let n = (m.shortMdctSize * (1 << lm)) as usize;
        let mut x = vec![0.0f32; n];
        let masks = [0u8; 32];
        /* the decoder's `oldEBands` arrays are `2*nbEBands` (celt.h) */
        let log_e = vec![-5.0f32; 2 * m.nbEBands as usize];
        let pulses = vec![8i32; m.nbEBands as usize];
        anti_collapse(
            &m,
            &mut x,
            &masks,
            lm,
            1,
            n as i32,
            0,
            4,
            &log_e,
            &log_e,
            &log_e,
            &pulses,
            1234,
            0,
            4,
        );
        assert!(x.iter().any(|v| *v != 0.0));
    }

    #[test]
    fn spreading_decision_range() {
        let m = mode();
        let n = (m.shortMdctSize * 2) as usize;
        let mut x = vec![0.0f32; n];
        let mut st: u32 = 3;
        for v in x.iter_mut() {
            st = st.wrapping_mul(1103515245).wrapping_add(12345);
            *v = ((st >> 8) as i32 - 8388608) as f32 / 8388608.0;
        }
        let w = vec![1i32; m.nbEBands as usize];
        let mut avg = 0;
        let mut hf = 0;
        let mut tap = 0;
        let d = spreading_decision(
            &m,
            &x,
            &mut avg,
            0,
            &mut hf,
            &mut tap,
            1,
            m.nbEBands,
            1,
            2,
            &w,
        );
        assert!((0..=3).contains(&d));
    }
}

//! Mirror of `celt/vq.{c,h}`.
/* Copyright (c) 2007-2008 CSIRO
   Copyright (c) 2007-2009 Xiph.Org Foundation
   Written by Jean-Marc Valin
   @file vq.h
   @brief Vector quantisation of the residual */
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
//!
//! This file is a transliteration of `celt/vq.c` and `celt/vq.h` (libopus
//! 1.6.1) into safe Rust.  The C names and the operation order are preserved
//! verbatim; only the pointer arguments became references.
//!
//! Configuration: the reference build is the **float** configuration
//! (`FIXED_POINT` and `FLOAT_APPROX` undefined), with x86 intrinsics and
//! `OPUS_X86_PRESUME_SSE2` defined.  Therefore:
//!
//! * the `FIXED_POINT` bodies of `normalise_residual()`, `renormalise_vector()`
//!   and `stereo_itheta()` are dropped.  `k` (which only exists under
//!   `FIXED_POINT`) disappears with them, and the float macros `VSHR32`,
//!   `PSHR32`, `EXTRACT16` ignore their shift argument, so those statements
//!   are plain multiplies here;
//! * `norm_scaleup()` / `norm_scaledown()` are the no-op macros of
//!   `vq.h:54-55`;
//! * `celt_inner_prod_norm` / `celt_inner_prod_norm_shift` are the macros of
//!   `vq.h:52-53`, i.e. `celt_inner_prod`, which the x86 build wires to
//!   **`celt_inner_prod_sse`** at every arch.  [`kernels::celt_inner_prod`] is
//!   that SSE kernel;
//! * `op_pvq_search(X,iy,K,N,arch)` is the `vq.h:63` macro, which
//!   `x86/vq_sse.h:34-38` overrides to `op_pvq_search_sse2` because
//!   `OPUS_X86_PRESUME_SSE2` is defined.  It is [`kernels::op_pvq_search`],
//!   which ignores `arch`, exactly like the C.  `op_pvq_search_c()` is *not*
//!   reachable in this configuration (it is compiled into `vq.o` as a global
//!   symbol, but no relocation refers to it) and is not duplicated here: the
//!   verified plain-C transliteration lives in [`kernels::op_pvq_search_c`];
//! * `ENABLE_QEXT` is undefined, so the `ARG_QEXT` parameters and the whole
//!   `op_pvq_search_N2`/`op_pvq_search_extra`/`cubic_*` block are dropped;
//! * `compute_qn()` does **not** live in this file (it is `celt/bands.c:638`).
//!
//! Verification: `compat/c/probe_vq.c` and `examples/probe_vq.rs` run the
//! identical deterministic sequence against the reference library and the
//! port; `bash compat/scripts/run-probe.sh vq` must print `IDENTICAL`.

use super::arch::*;
use super::celt::MAX_PVQ_N;
use super::cwrs::{decode_pulses, encode_pulses};
use super::entcode::{celt_udiv, ec_ctx};
use super::kernels;
use super::mathops::{
    celt_atan2p_norm, celt_cos_norm, celt_div, celt_rsqrt_norm, celt_rsqrt_norm32, celt_sqrt32,
};
use crate::types::*;

/* `celt/bands.h:68` — `bands.c` is ported in its own module. */
const SPREAD_NONE: i32 = 0;

// ---------------------------------------------------------------------------
// `vq.h:44-56`: the float build defines these away.
// ---------------------------------------------------------------------------

/// `#define norm_scaleup(X, N, shift)` (float build: no-op).
#[inline(always)]
fn norm_scaleup(_X: &mut [celt_norm], _N: i32, _shift: i32) {}

/// `#define norm_scaledown(X, N, shift)` (float build: no-op).
#[inline(always)]
fn norm_scaledown(_X: &mut [celt_norm], _N: i32, _shift: i32) {}

/// `#define celt_inner_prod_norm celt_inner_prod` (float build).
///
/// `celt_inner_prod` is overridden by `x86/pitch_sse.h:112-115` to
/// `celt_inner_prod_sse` at every arch, i.e. [`kernels::celt_inner_prod`].
#[inline(always)]
fn celt_inner_prod_norm(x: &[celt_norm], y: &[celt_norm], len: i32, arch: i32) -> opus_val32 {
    /* `((void)arch, celt_inner_prod_sse(x, y, N))` */
    let _ = arch;
    kernels::celt_inner_prod(x, y, len)
}

/// `#define celt_inner_prod_norm_shift celt_inner_prod` (float build).
#[inline(always)]
fn celt_inner_prod_norm_shift(
    x: &[celt_norm],
    y: &[celt_norm],
    len: i32,
    arch: i32,
) -> opus_val32 {
    let _ = arch;
    kernels::celt_inner_prod(x, y, len)
}

/* `#ifndef OVERRIDE_vq_exp_rotation1` — x86 does not override it. */
/// `celt/vq.c:75` `exp_rotation1()`.
///
/// The two loops are the *same* read-modify-write sequence as the C: within one
/// iteration `x1`/`x2` are read before either `X[i]` or `X[i+stride]` is
/// written, and the second loop walks downward over values the higher-index
/// iterations have already produced, so the iteration order is part of the
/// arithmetic.
fn exp_rotation1(X: &mut [celt_norm], len: i32, stride: i32, c: opus_val16, s: opus_val16) {
    let ms = NEG16(s);
    norm_scaledown(X, len, NORM_SHIFT - 14);
    let mut i = 0;
    while i < len - stride {
        let x1 = X[i as usize];
        let x2 = X[(i + stride) as usize];
        X[(i + stride) as usize] = EXTRACT16(PSHR32(MAC16_16(MULT16_16(c, x2), s, x1), 15));
        X[i as usize] = EXTRACT16(PSHR32(MAC16_16(MULT16_16(c, x1), ms, x2), 15));
        i += 1;
    }
    /* `Xptr = &X[len-2*stride-1];` — that pointer is only formed, never
       dereferenced, when the index is negative, so the loop is simply skipped
       there (the C would be undefined, not observably different). */
    let mut i = len - 2 * stride - 1;
    while i >= 0 {
        let x1 = X[i as usize];
        let x2 = X[(i + stride) as usize];
        X[(i + stride) as usize] = EXTRACT16(PSHR32(MAC16_16(MULT16_16(c, x2), s, x1), 15));
        X[i as usize] = EXTRACT16(PSHR32(MAC16_16(MULT16_16(c, x1), ms, x2), 15));
        i -= 1;
    }
    norm_scaleup(X, len, NORM_SHIFT - 14);
}

/// `celt/vq.c:104` `exp_rotation()`.
pub fn exp_rotation(X: &mut [celt_norm], len: i32, dir: i32, stride: i32, K: i32, spread: i32) {
    static SPREAD_FACTOR: [i32; 3] = [15, 10, 5];
    let mut stride2 = 0;
    let c;
    let s;

    if 2 * K >= len || spread == SPREAD_NONE {
        return;
    }
    let factor = SPREAD_FACTOR[(spread - 1) as usize];

    let gain = celt_div(
        MULT16_16(Q15_ONE, len as f32),
        (len + factor * K) as opus_val32,
    );
    let theta = HALF16(MULT16_16_Q15(gain, gain));

    c = celt_cos_norm(EXTEND32(theta));
    s = celt_cos_norm(EXTEND32(SUB16(Q15ONE, theta))); /*  sin(theta) */

    if len >= 8 * stride {
        stride2 = 1;
        /* This is just a simple (equivalent) way of computing sqrt(len/stride)
           with rounding.  It's basically incrementing long as
           (stride2+0.5)^2 < len/stride. */
        while (stride2 * stride2 + stride2) * stride + (stride >> 2) < len {
            stride2 += 1;
        }
    }
    /*NOTE: As a minor optimization, we could be passing around log2(B), not B,
      for both this and for extract_collapse_mask().*/
    let len = celt_udiv(len as opus_uint32, stride as opus_uint32) as i32;
    for i in 0..stride {
        let off = (i * len) as usize;
        if dir < 0 {
            if stride2 != 0 {
                exp_rotation1(&mut X[off..], len, stride2, s, c);
            }
            exp_rotation1(&mut X[off..], len, 1, c, s);
        } else {
            exp_rotation1(&mut X[off..], len, 1, c, -s);
            if stride2 != 0 {
                exp_rotation1(&mut X[off..], len, stride2, s, -c);
            }
        }
    }
}

/// `celt/vq.c:150` `normalise_residual()` — float branch.
///
/// The `FIXED_POINT` body computes `k = celt_ilog2(Ryy)>>1` and uses it in the
/// `VSHR32()` shift arguments; in the float build `VSHR32(a,shift) == a`, so
/// neither `k` nor `shift` is used.
fn normalise_residual(
    iy: &[i32],
    X: &mut [celt_norm],
    N: i32,
    Ryy: opus_val32,
    gain: opus_val32,
    shift: i32,
) {
    let _ = shift;
    /* `t = VSHR32(Ryy, 2*(k-7)-15)` with the float `VSHR32()`. */
    let t = VSHR32(Ryy, 0);
    let g = MULT32_32_Q31(celt_rsqrt_norm32(t), gain);
    let mut i: i32 = 0;
    loop {
        X[i as usize] = VSHR32(MULT16_32_Q15(iy[i as usize] as opus_val32, g), 0);
        i += 1;
        if i >= N {
            break;
        }
    }
}

/// `celt/vq.c:183` `extract_collapse_mask()`.
fn extract_collapse_mask(iy: &[i32], N: i32, B: i32) -> opus_uint32 {
    if B <= 1 {
        return 1;
    }
    /*NOTE: As a minor optimization, we could be passing around log2(B), not B,
      for both this and for exp_rotation().*/
    let N0 = celt_udiv(N as opus_uint32, B as opus_uint32) as i32;
    let mut collapse_mask: opus_uint32 = 0;
    let mut i: i32 = 0;
    loop {
        let mut tmp: opus_uint32 = 0;
        let mut j: i32 = 0;
        loop {
            /* `tmp |= iy[i*N0+j]` — the `int` is converted to `unsigned`. */
            tmp |= iy[(i * N0 + j) as usize] as opus_uint32;
            j += 1;
            if j >= N0 {
                break;
            }
        }
        /* `(tmp!=0)<<i` on C `int`; `wrapping_shl` keeps an out-of-range `i`
           (unreachable for valid input) from panicking in debug builds. */
        collapse_mask |= ((tmp != 0) as opus_uint32).wrapping_shl(i as u32);
        i += 1;
        if i >= B {
            break;
        }
    }
    collapse_mask
}

/* `op_pvq_search_c()` (`celt/vq.c:205`) is compiled into the reference `vq.o`
   but is unreachable in this configuration: `OPUS_X86_PRESUME_SSE2` makes the
   `op_pvq_search` macro below expand to `op_pvq_search_sse2`, and `objdump -r
   vq.o` shows a single relocation, `op_pvq_search_sse2`.  The verified
   plain-C transliteration therefore lives only in
   `crate::celt::kernels::op_pvq_search_c` (used by the kernels probe). */

/// `celt/vq.c:550` `alg_quant()` — float, non-`ENABLE_QEXT` body.
pub fn alg_quant(
    X: &mut [celt_norm],
    N: i32,
    K: i32,
    spread: i32,
    B: i32,
    enc: &mut ec_ctx,
    gain: opus_val32,
    resynth: i32,
    arch: i32,
) -> opus_uint32 {
    celt_assert2!(K > 0, "alg_quant() needs at least one pulse");
    celt_assert2!(N > 1, "alg_quant() needs at least two dimensions");

    /* Covers vectorization by up to 4.  The C `VARDECL(int, iy)` is sized by
       the mode (`N <= MAX_PVQ_N`), so this is stack storage; the `+3` covers
       the 4-wide stores `op_pvq_search` performs. */
    let mut iy_storage = [0i32; MAX_PVQ_N + 3];
    let iy = &mut iy_storage[..(N + 3) as usize];

    exp_rotation(X, N, 1, B, K, spread);

    /* `yy = op_pvq_search(X, iy, K, N, arch);` — the `vq.h:63` macro, which
       `x86/vq_sse.h:36-37` overrides to `op_pvq_search_sse2`. */
    let yy = kernels::op_pvq_search(X, iy, K, N, arch);
    let collapse_mask = extract_collapse_mask(iy, N, B);
    encode_pulses(iy, N, K, enc);
    if resynth != 0 {
        normalise_residual(iy, X, N, yy, gain, 0);
    }

    if resynth != 0 {
        exp_rotation(X, N, -1, B, K, spread);
    }

    collapse_mask
}

/// `celt/vq.c:619` `alg_unquant()` — float, non-`ENABLE_QEXT` body.
pub fn alg_unquant(
    X: &mut [celt_norm],
    N: i32,
    K: i32,
    spread: i32,
    B: i32,
    dec: &mut ec_ctx,
    gain: opus_val32,
) -> opus_uint32 {
    celt_assert2!(K > 0, "alg_unquant() needs at least one pulse");
    celt_assert2!(N > 1, "alg_unquant() needs at least two dimensions");

    /* `VARDECL(int, iy); ALLOC(iy, N, int);` — `N <= MAX_PVQ_N`, stack. */
    let mut iy_storage = [0i32; MAX_PVQ_N];
    let iy = &mut iy_storage[..N as usize];
    let Ryy = decode_pulses(iy, N, K, dec);
    normalise_residual(iy, X, N, Ryy, gain, 0);
    exp_rotation(X, N, -1, B, K, spread);
    extract_collapse_mask(iy, N, B)
}

/* `#ifndef OVERRIDE_renormalise_vector` — x86 does not override it. */
/// `celt/vq.c:693` `renormalise_vector()` — float branch.
pub fn renormalise_vector(X: &mut [celt_norm], N: i32, gain: opus_val32, arch: i32) {
    norm_scaledown(X, N, NORM_SHIFT - 14);
    let E = EPSILON + celt_inner_prod_norm(X, X, N, arch);
    /* `k = celt_ilog2(E)>>1` only exists under FIXED_POINT; the float
       `VSHR32()` ignores its shift. */
    let t = VSHR32(E, 0);
    let g = MULT32_32_Q31(celt_rsqrt_norm(t), gain);

    for i in 0..N as usize {
        X[i] = EXTRACT16(PSHR32(MULT16_16(g, X[i]), 0));
    }
    norm_scaleup(X, N, NORM_SHIFT - 14);
    /*return celt_sqrt(E);*/
}

/// `celt/vq.c:722` `stereo_itheta()`.
pub fn stereo_itheta(
    X: &[celt_norm],
    Y: &[celt_norm],
    stereo: i32,
    N: i32,
    arch: i32,
) -> opus_int32 {
    let itheta: opus_int32;
    let mid;
    let side;
    let mut Emid: opus_val32 = 0.;
    let mut Eside: opus_val32 = 0.;

    if stereo != 0 {
        for i in 0..N as usize {
            let m: celt_norm;
            let s: celt_norm;
            m = PSHR32(ADD32(X[i], Y[i]), NORM_SHIFT - 13);
            s = PSHR32(SUB32(X[i], Y[i]), NORM_SHIFT - 13);
            Emid = MAC16_16(Emid, m, m);
            Eside = MAC16_16(Eside, s, s);
        }
    } else {
        Emid += celt_inner_prod_norm_shift(X, X, N, arch);
        Eside += celt_inner_prod_norm_shift(Y, Y, N, arch);
    }
    mid = celt_sqrt32(Emid);
    side = celt_sqrt32(Eside);

    itheta = (0.5f32 + 65536.0f32 * 16384.0 * celt_atan2p_norm(side, mid)).floor() as opus_int32;

    itheta
}

/* `ENABLE_QEXT` is undefined in the reference configuration, so
   `cubic_quant()`, `cubic_unquant()`, `op_pvq_search_N2()`,
   `op_pvq_search_extra()` and `op_pvq_refine()` are not compiled and are
   dropped here. */

// ---------------------------------------------------------------------------
// Unit tests for the pure helpers.
//
// Bit-exactness against the C library is proven by
// `bash compat/scripts/run-probe.sh vq`; these tests only pin down the
// self-contained behaviour of the helpers that have an exact closed form.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `extract_collapse_mask` returns 1 for a single block and otherwise
    /// reports, per block, whether that block received a non-zero pulse.
    #[test]
    fn collapse_mask() {
        assert_eq!(extract_collapse_mask(&[0, 0], 2, 1), 1);
        assert_eq!(extract_collapse_mask(&[0, 1], 2, 1), 1);
        /* B=4, N=8 => N0=2: blocks are iy[0..2], [2..4], [4..6], [6..8]. */
        assert_eq!(extract_collapse_mask(&[1, 0, 0, 0, 0, 0, 0, 0], 8, 4), 0b0001);
        assert_eq!(extract_collapse_mask(&[0, 0, 1, 0, 0, 0, 0, 0], 8, 4), 0b0010);
        assert_eq!(extract_collapse_mask(&[0, 0, 0, 0, 0, -1, 0, 0], 8, 4), 0b0100);
        assert_eq!(extract_collapse_mask(&[0, 0, 0, 0, 0, 0, 0, 2], 8, 4), 0b1000);
        assert_eq!(extract_collapse_mask(&[1, 1, 1, 1, 1, 1, 1, 1], 8, 4), 0b1111);
        /* A negative pulse counts as non-zero (`tmp |= (unsigned)iy[...]`). */
        assert_eq!(extract_collapse_mask(&[-1, 0, 0, 0], 4, 2), 0b01);
    }

    /// `exp_rotation` returns immediately when `2*K >= len` and when
    /// `spread == SPREAD_NONE`.
    #[test]
    fn exp_rotation_early_out() {
        let orig: Vec<f32> = (0..16).map(|i| (i as f32) * 0.125 - 1.0).collect();
        let mut x = orig.clone();
        exp_rotation(&mut x, 16, 1, 1, 8, 2); /* 2*K == len */
        assert_eq!(x, orig);
        let mut x = orig.clone();
        exp_rotation(&mut x, 16, 1, 1, 1, SPREAD_NONE);
        assert_eq!(x, orig);
    }

    /// With `len == 2` the downward loop has `len-2*stride-1 == -1` and is
    /// skipped, so `exp_rotation1` is exactly one plane rotation:
    /// `X[0] = c*X[0] - s*X[1]`, `X[1] = c*X[1] + s*X[0]`.
    #[test]
    fn exp_rotation1_len2() {
        let (c, s) = (0.6f32, 0.8f32);
        let mut x = [1.0f32, 2.0];
        exp_rotation1(&mut x, 2, 1, c, s);
        assert_eq!(x, [c * 1.0 - s * 2.0, c * 2.0 + s * 1.0]);
    }

    /// `exp_rotation1` is a composition of plane rotations, hence
    /// norm-preserving (up to rounding).
    #[test]
    fn exp_rotation1_preserves_norm() {
        let orig = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut x = orig;
        exp_rotation1(&mut x, 6, 1, 0.6, 0.8);
        let n0: f32 = orig.iter().map(|v| v * v).sum();
        let n1: f32 = x.iter().map(|v| v * v).sum();
        assert!((n0 - n1).abs() < 1e-4, "n0={n0} n1={n1}");
    }

    /// A unit pulse on the first dimension renormalises to one with gain 1.
    #[test]
    fn normalise_residual_unit_pulse() {
        let mut x = [0.0f32; 4];
        normalise_residual(&[1, 0, 0, 0], &mut x, 4, 1.0, 1.0, 0);
        assert_eq!(x, [1.0, 0.0, 0.0, 0.0]);
        /* <iy,iy> == Ryy == 4 and gain 1: g = 1/sqrt(4) = 0.5. */
        let mut x = [0.0f32; 4];
        normalise_residual(&[2, 0, 0, 0], &mut x, 4, 4.0, 1.0, 0);
        assert_eq!(x, [1.0, 0.0, 0.0, 0.0]);
    }

    /// `renormalise_vector` scales to `gain` and never divides by zero thanks
    /// to `EPSILON`: an all-zero vector stays all-zero.
    #[test]
    fn renormalise_vector_scales() {
        let mut x = [3.0f32, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        renormalise_vector(&mut x, 8, 2.0, 4);
        assert!((x[0] - 1.2).abs() < 1e-6 && (x[1] - 1.6).abs() < 1e-6);
        let mut z = [0.0f32; 4];
        renormalise_vector(&mut z, 4, 1.0, 4);
        assert_eq!(z, [0.0f32; 4]);
    }

    /// `stereo_itheta` with `X == Y`: in the `stereo` branch the side signal is
    /// exactly zero, so the angle is 0; in the dual-mono branch the two
    /// energies are equal, so `celt_atan2p_norm(1,1) == 0.5` and the result is
    /// `floor(.5 + 2^30 * .5) == 2^29`.
    #[test]
    fn stereo_itheta_identical_channels() {
        let x = [0.5f32, -0.25, 0.75, 0.125];
        assert_eq!(stereo_itheta(&x, &x, 1, 4, 4), 0);
        assert_eq!(stereo_itheta(&x, &x, 0, 4, 4), 1 << 29);
    }

    /// A mid/side split with no mid energy gives the maximum angle:
    /// `celt_atan2p_norm(1, 0) == 1` and `floor(.5 + 2^30)` is `2^30`.
    #[test]
    fn stereo_itheta_pure_side() {
        let x = [1.0f32, 0.0, 0.0, 0.0];
        let y = [-1.0f32, 0.0, 0.0, 0.0];
        /* stereo=1: m = 0, s = 2 on dim 0 => Emid=0, Eside=4. */
        assert_eq!(stereo_itheta(&x, &y, 1, 4, 4), 1 << 30);
    }
}

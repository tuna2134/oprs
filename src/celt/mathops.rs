//! Transliteration of `celt/mathops.{h,c}` for the **float** configuration
//! (`FIXED_POINT` undefined, `FLOAT_APPROX` undefined, `ENABLE_QEXT`
//! undefined), which is the configuration the reference `libopus` static
//! library is built with.
//!
//! ```text
//! Copyright (c) 2002-2008 Jean-Marc Valin
//! Copyright (c) 2007-2008 CSIRO
//! Copyright (c) 2007-2009 Xiph.Org Foundation
//! Copyright (c) 2024 Arm Limited
//! Written by Jean-Marc Valin, and Yunho Huh
//! ```
//!
//! This file is a transliteration of the above BSD-3-Clause sources; the
//! original license follows.
//!
//! ```text
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions
//! are met:
//!
//! - Redistributions of source code must retain the above copyright
//! notice, this list of conditions and the following disclaimer.
//!
//! - Redistributions in binary form must reproduce the above copyright
//! notice, this list of conditions and the following disclaimer in the
//! documentation and/or other materials provided with the distribution.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
//! ``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
//! LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
//! A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
//! OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
//! PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
//! PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
//! LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//! ```
//!
//! Only the branches that are live in the pinned configuration are ported:
//! the `#ifdef FIXED_POINT` bodies of `mathops.h`/`mathops.c` (the Q15/Q31
//! polynomial approximations of `celt_sqrt`, `celt_rsqrt_norm`, `celt_rcp`,
//! `celt_exp2_frac`, `celt_atan2p`, …) are dead here and intentionally
//! omitted.  In the float build those names are macros over libm:
//!
//! ```c
//! #define celt_sqrt(x)        ((float)sqrt(x))
//! #define celt_rsqrt(x)       (1.f/celt_sqrt(x))
//! #define celt_rsqrt_norm(x)  (celt_rsqrt(x))
//! #define celt_cos_norm(x)    ((float)cos((.5f*PI)*(x)))
//! #define celt_rcp(x)         (1.f/(x))
//! #define celt_div(a,b)       ((a)/(b))
//! #define frac_div32(a,b)     ((float)(a)/(b))
//! #define frac_div32_q29(a,b) frac_div32(a,b)
//! #define celt_log2(x)        ((float)(1.442695040888963387*log(x)))
//! #define celt_exp2(x)        ((float)exp(0.6931471805599453094*(x)))
//! #define celt_sin(x)         celt_cos_norm2((0.5f*PI) * (x) - 1.0f)
//! #define celt_log(x)         (celt_log2(x) * 0.6931471805599453f)
//! #define celt_exp(x)         (celt_exp2((x) * 1.4426950408889634f))
//! ```
//!
//! `celt_lcg_rand`, `bitexact_cos` and `bitexact_log2tan` are defined in
//! `celt/bands.c` upstream, not in `mathops.c`; they are kept here (with their
//! true origin noted at each definition) because they are part of the
//! `mathops` verification set.
//!
//! The C source spells out the numeric constants (`PI`, `ATAN2_2_OVER_PI`,
//! `1.442695040888963387`, …) literally; they are kept verbatim here even
//! where clippy would prefer the `f32::consts`/`f64::consts` constant, so that
//! the port can still be diffed line by line against `mathops.h`.
#![allow(clippy::approx_constant)]

use crate::celt::arch::*;
use crate::celt::entcode::EC_ILOG;
use crate::types::*;

/// `PI`
pub const PI: f64 = 3.1415926535897931;

/* Multiplies two 16-bit fractional values. Bit-exactness of this macro is important */
/// `FRAC_MUL16(a,b)`
#[inline(always)]
pub fn FRAC_MUL16(a: opus_int32, b: opus_int32) -> opus_int32 {
    (16384 + ((a as opus_int16 as opus_int32) * (b as opus_int16 as opus_int32))) >> 15
}

/// `isqrt32`: compute `floor(sqrt(_val))` with exact arithmetic.
///
/// `_val` must be greater than 0 (the C function's `1U<<bshift` is undefined
/// for `_val == 0`); this is tested on all 32-bit inputs greater than 0.
pub fn isqrt32(_val: opus_uint32) -> u32 {
    let mut _val = _val;
    let mut b: u32;
    let mut g: u32;
    let mut bshift: i32;
    /*Uses the second method from
     http://www.azillionmonkeys.com/qed/sqroot.html
    The main idea is to search for the largest binary digit b such that
     (g+b)*(g+b) <= _val, and add it to the solution g.*/
    g = 0;
    bshift = (EC_ILOG(_val) - 1) >> 1;
    b = 1u32 << bshift;
    loop {
        let t: opus_uint32;
        t = ((g << 1).wrapping_add(b)) << bshift;
        if t <= _val {
            g += b;
            _val -= t;
        }
        b >>= 1;
        bshift -= 1;
        if bshift < 0 {
            break;
        }
    }
    g
}

/// `fast_atan2f` — active in the float build (`!defined(FIXED_POINT)`).
#[inline(always)]
pub fn fast_atan2f(y: f32, x: f32) -> f32 {
    let cA: f32 = 0.43157974f32;
    let cB: f32 = 0.67848403f32;
    let cC: f32 = 0.08595542f32;
    let cE: f32 = (PI as f32) / 2.0;
    let x2: f32;
    let y2: f32;
    x2 = x * x;
    y2 = y * y;
    /* For very small values, we don't care about the answer, so
    we can just return 0. */
    if x2 + y2 < 1e-18f32 {
        return 0.0;
    }
    if x2 < y2 {
        let den: f32 = (y2 + cB * x2) * (y2 + cC * x2);
        -x * y * (y2 + cA * x2) / den + (if y < 0.0 { -cE } else { cE })
    } else {
        let den: f32 = (x2 + cB * y2) * (x2 + cC * y2);
        x * y * (x2 + cA * y2) / den + (if y < 0.0 { -cE } else { cE })
            - (if x * y < 0.0 { -cE } else { cE })
    }
}

/// `celt_maxabs16`
#[inline(always)]
pub fn celt_maxabs16(x: &[opus_val16], len: i32) -> opus_val32 {
    let mut i: i32;
    let mut maxval: opus_val16 = 0.0;
    let mut minval: opus_val16 = 0.0;
    i = 0;
    while i < len {
        maxval = MAX16(maxval, x[i as usize]);
        minval = MIN16(minval, x[i as usize]);
        i += 1;
    }
    MAX32(EXTEND32(maxval), -EXTEND32(minval))
}

/// `#define celt_maxabs_res celt_maxabs16` (`ENABLE_RES24` undefined)
#[inline(always)]
pub fn celt_maxabs_res(x: &[opus_res], len: i32) -> opus_val32 {
    celt_maxabs16(x, len)
}

/// `#define celt_maxabs32(x,len) celt_maxabs16(x,len)` (float build)
#[inline(always)]
pub fn celt_maxabs32(x: &[opus_val32], len: i32) -> opus_val32 {
    celt_maxabs16(x, len)
}

// ---------------------------------------------------------------------------
// `#ifndef FIXED_POINT`
// ---------------------------------------------------------------------------

/* Calculates the arctangent of x using a Remez approximation of order 15,
 * incorporating only odd-powered terms. */
/// `celt_atan_norm`
#[inline(always)]
pub fn celt_atan_norm(x: f32) -> f32 {
    const ATAN2_2_OVER_PI: f32 = 0.636619772367581f32;
    let x_sq: f32 = x * x;

    /* Polynomial coefficients approximated in the [0, 1] range.
     * Lolremez command: lolremez --degree 6 --range "0:1"
     *                   "(atan(sqrt(x))-sqrt(x))/(x*sqrt(x))" "1/(sqrt(x)*x)"
     * Please note that ATAN2_COEFF_A01 is fixed to 1.0f. */
    const ATAN2_COEFF_A03: f32 = -3.3331659436225891113281250000e-01f32;
    const ATAN2_COEFF_A05: f32 = 1.99627041816711425781250000000e-01f32;
    const ATAN2_COEFF_A07: f32 = -1.3976582884788513183593750000e-01f32;
    const ATAN2_COEFF_A09: f32 = 9.79423448443412780761718750000e-02f32;
    const ATAN2_COEFF_A11: f32 = -5.7773590087890625000000000000e-02f32;
    const ATAN2_COEFF_A13: f32 = 2.30401363223791122436523437500e-02f32;
    const ATAN2_COEFF_A15: f32 = -4.3554059229791164398193359375e-03f32;
    ATAN2_2_OVER_PI
        * (x + x
            * x_sq
            * (ATAN2_COEFF_A03
                + x_sq
                    * (ATAN2_COEFF_A05
                        + x_sq
                            * (ATAN2_COEFF_A07
                                + x_sq
                                    * (ATAN2_COEFF_A09
                                        + x_sq
                                            * (ATAN2_COEFF_A11
                                                + x_sq
                                                    * (ATAN2_COEFF_A13
                                                        + x_sq * (ATAN2_COEFF_A15))))))))
}

/* Calculates the arctangent of y/x, returning an approximate value in radians.
 * Please refer to the linked wiki page (https://en.wikipedia.org/wiki/Atan2)
 * to learn how atan2 results are computed. */
/// `celt_atan2p_norm`
#[inline(always)]
pub fn celt_atan2p_norm(y: f32, x: f32) -> f32 {
    celt_sig_assert!(x >= 0.0 && y >= 0.0);

    /* For very small values, we don't care about the answer. */
    if (x * x + y * y) < 1e-18f32 {
        return 0.0;
    }

    if y < x {
        celt_atan_norm(y / x)
    } else {
        1.0f32 - celt_atan_norm(x / y)
    }
}

/* Computes estimated cosine values for (PI/2 * x) using only terms with even
 * exponents. */
/// `celt_cos_norm2` (float build: `!defined(FIXED_POINT) || defined(ENABLE_QEXT)`)
#[inline(always)]
pub fn celt_cos_norm2(x: f32) -> f32 {
    let mut x = x;
    let x_norm_sq: f32;
    let output_sign: i32;
    /* Restrict x to [-1, 3]. */
    x = (x as f64 - 4.0 * (0.25 * ((x + 1.0f32) as f64)).floor()) as f32;
    /* Negative sign for [1, 3]. */
    output_sign = 1 - 2 * (x > 1.0f32) as i32;
    /* Restrict to [-1, 1]. */
    x = x - (2 * (x > 1.0f32) as i32) as f32;

    /* The cosine function, cos(x), has a Taylor series representation consisting
     * exclusively of even-powered polynomial terms. */
    x_norm_sq = x * x;

    /* Polynomial coefficients approximated in the [0, 1] range using only terms
     * with even exponents.
     * Lolremez command: lolremez --degree 4 --range 0:1 "cos(sqrt(x)*pi*0.5)" */
    const COS_COEFF_A0: f32 = 9.999999403953552246093750000000e-01f32;
    const COS_COEFF_A2: f32 = -1.233698248863220214843750000000000f32;
    const COS_COEFF_A4: f32 = 2.536507546901702880859375000000e-01f32;
    const COS_COEFF_A6: f32 = -2.08106283098459243774414062500e-02f32;
    const COS_COEFF_A8: f32 = 8.581906440667808055877685546875e-04f32;
    output_sign as f32
        * (COS_COEFF_A0
            + x_norm_sq
                * (COS_COEFF_A2
                    + x_norm_sq
                        * (COS_COEFF_A4 + x_norm_sq * (COS_COEFF_A6 + x_norm_sq * (COS_COEFF_A8)))))
}

// ---------------------------------------------------------------------------
// Float-build macros from `mathops.h`.
// ---------------------------------------------------------------------------

/// `#define celt_sqrt(x) ((float)sqrt(x))`
#[inline(always)]
pub fn celt_sqrt(x: opus_val32) -> opus_val32 {
    x.sqrt()
}

/// `#define celt_sqrt32(x) ((float)sqrt(x))`
#[inline(always)]
pub fn celt_sqrt32(x: opus_val32) -> opus_val32 {
    x.sqrt()
}

/// `#define celt_rsqrt(x) (1.f/celt_sqrt(x))`
#[inline(always)]
pub fn celt_rsqrt(x: opus_val32) -> opus_val32 {
    1.0f32 / celt_sqrt(x)
}

/// `#define celt_rsqrt_norm(x) (celt_rsqrt(x))`
#[inline(always)]
pub fn celt_rsqrt_norm(x: opus_val32) -> opus_val16 {
    celt_rsqrt(x)
}

/// `#define celt_rsqrt_norm32(x) (celt_rsqrt(x))`
#[inline(always)]
pub fn celt_rsqrt_norm32(x: opus_val32) -> opus_val32 {
    celt_rsqrt(x)
}

/// `#define celt_cos_norm(x) ((float)cos((.5f*PI)*(x)))`
#[inline(always)]
pub fn celt_cos_norm(x: opus_val32) -> opus_val16 {
    ((0.5f32 as f64 * PI) * (x as f64)).cos() as f32
}

/// `#define celt_rcp(x) (1.f/(x))`
#[inline(always)]
pub fn celt_rcp(x: opus_val32) -> opus_val32 {
    1.0f32 / x
}

/// `#define celt_div(a,b) ((a)/(b))`
#[inline(always)]
pub fn celt_div(a: opus_val32, b: opus_val32) -> opus_val32 {
    a / b
}

/// `#define frac_div32(a,b) ((float)(a)/(b))`
#[inline(always)]
pub fn frac_div32(a: opus_val32, b: opus_val32) -> opus_val32 {
    a / b
}

/// `#define frac_div32_q29(a,b) frac_div32(a,b)`
#[inline(always)]
pub fn frac_div32_q29(a: opus_val32, b: opus_val32) -> opus_val32 {
    frac_div32(a, b)
}

/// `#define celt_log2(x) ((float)(1.442695040888963387*log(x)))`
/// (`FLOAT_APPROX` undefined)
#[inline(always)]
pub fn celt_log2(x: opus_val32) -> opus_val16 {
    (1.442695040888963387f64 * (x as f64).ln()) as f32
}

/// `#define celt_exp2(x) ((float)exp(0.6931471805599453094*(x)))`
/// (`FLOAT_APPROX` undefined)
#[inline(always)]
pub fn celt_exp2(x: opus_val16) -> opus_val32 {
    (0.6931471805599453094f64 * (x as f64)).exp() as f32
}

/// `#define celt_exp2_db celt_exp2`
#[inline(always)]
pub fn celt_exp2_db(x: opus_val16) -> opus_val32 {
    celt_exp2(x)
}

/// `#define celt_log2_db celt_log2`
#[inline(always)]
pub fn celt_log2_db(x: opus_val32) -> opus_val16 {
    celt_log2(x)
}

/// `#define celt_sin(x) celt_cos_norm2((0.5f*PI) * (x) - 1.0f)`
#[inline(always)]
pub fn celt_sin(x: opus_val32) -> f32 {
    celt_cos_norm2(((0.5f32 as f64 * PI) * (x as f64) - 1.0f64) as f32)
}

/// `#define celt_log(x) (celt_log2(x) * 0.6931471805599453f)`
#[inline(always)]
pub fn celt_log(x: opus_val32) -> opus_val16 {
    celt_log2(x) * 0.6931471805599453f32
}

/// `#define celt_exp(x) (celt_exp2((x) * 1.4426950408889634f))`
#[inline(always)]
pub fn celt_exp(x: opus_val16) -> opus_val32 {
    celt_exp2(x * 1.4426950408889634f32)
}

// ---------------------------------------------------------------------------
// Helpers that live in `celt/bands.c` upstream.
//
// They are grouped here with the `mathops` verification set; the C definitions
// are in `celt/bands.c` (`celt_lcg_rand`, `bitexact_cos`, `bitexact_log2tan`).
// ---------------------------------------------------------------------------

/// `celt/bands.c`: `celt_lcg_rand`
pub fn celt_lcg_rand(seed: opus_uint32) -> opus_uint32 {
    1664525u32.wrapping_mul(seed).wrapping_add(1013904223u32)
}

/* This is a cos() approximation designed to be bit-exact on any platform. Bit exactness
with this approximation is important because it has an impact on the bit allocation */
/// `celt/bands.c`: `bitexact_cos`
pub fn bitexact_cos(x: opus_int16) -> opus_int16 {
    let tmp: opus_int32;
    let mut x2: opus_int16;
    tmp = (4096 + ((x as opus_int32) * (x as opus_int32))) >> 13;
    celt_sig_assert!(tmp <= 32767);
    x2 = tmp as opus_int16;
    x2 = (32767 - x2 as opus_int32
        + FRAC_MUL16(
            x2 as opus_int32,
            -7651 + FRAC_MUL16(x2 as opus_int32, 8277 + FRAC_MUL16(-626, x2 as opus_int32)),
        )) as opus_int16;
    celt_sig_assert!(x2 <= 32766);
    (1 + x2 as opus_int32) as opus_int16
}

/// `celt/bands.c`: `bitexact_log2tan`
pub fn bitexact_log2tan(isin: i32, icos: i32) -> i32 {
    let lc: i32;
    let ls: i32;
    lc = EC_ILOG(icos as opus_uint32);
    ls = EC_ILOG(isin as opus_uint32);
    let icos = icos.wrapping_shl((15 - lc) as u32);
    let isin = isin.wrapping_shl((15 - ls) as u32);
    (ls - lc) * (1 << 11) + FRAC_MUL16(isin, FRAC_MUL16(isin, -2597) + 7932)
        - FRAC_MUL16(icos, FRAC_MUL16(icos, -2597) + 7932)
}

// ---------------------------------------------------------------------------
// `#ifndef DISABLE_FLOAT_API`
// ---------------------------------------------------------------------------

/// `celt_float2int16_c`
pub fn celt_float2int16_c(in_: &[f32], out: &mut [opus_int16], cnt: i32) {
    let mut i: i32;
    i = 0;
    while i < cnt {
        out[i as usize] = FLOAT2INT16(in_[i as usize]);
        i += 1;
    }
}

/// `#define celt_float2int16(in, out, cnt, arch) ((void)(arch), celt_float2int16_c(in, out, cnt))`
#[inline(always)]
pub fn celt_float2int16(in_: &[f32], out: &mut [opus_int16], cnt: i32, _arch: i32) {
    celt_float2int16_c(in_, out, cnt)
}

/// `opus_limit2_checkwithin1_c`
pub fn opus_limit2_checkwithin1_c(samples: &mut [f32], cnt: i32) -> i32 {
    let mut i: i32;
    if cnt <= 0 {
        return 1;
    }

    i = 0;
    while i < cnt {
        let mut clippedVal: f32 = samples[i as usize];
        clippedVal = FMAX(-2.0f32, clippedVal);
        clippedVal = FMIN(2.0f32, clippedVal);
        samples[i as usize] = clippedVal;
        i += 1;
    }

    /* C implementation can't provide quick hint. Assume it might exceed -1/+1. */
    0
}

/// `#define opus_limit2_checkwithin1(samples, cnt, arch) ((void)(arch), opus_limit2_checkwithin1_c(samples, cnt))`
#[inline(always)]
pub fn opus_limit2_checkwithin1(samples: &mut [f32], cnt: i32, _arch: i32) -> i32 {
    opus_limit2_checkwithin1_c(samples, cnt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt32_exact() {
        assert_eq!(isqrt32(1), 1);
        assert_eq!(isqrt32(2), 1);
        assert_eq!(isqrt32(3), 1);
        assert_eq!(isqrt32(4), 2);
        assert_eq!(isqrt32(8), 2);
        assert_eq!(isqrt32(9), 3);
        assert_eq!(isqrt32(1000000), 1000);
        assert_eq!(isqrt32(1000001), 1000);
        assert_eq!(isqrt32(0xFFFFFFFF), 65535);
        assert_eq!(isqrt32(0x7FFFFFFF), 46340);
        assert_eq!(isqrt32(1 << 31), 46340);
    }

    #[test]
    fn lcg_rand_sequence() {
        /* 1664525*seed + 1013904223 (mod 2^32), the "Numerical Recipes" LCG. */
        assert_eq!(celt_lcg_rand(0), 1013904223);
        assert_eq!(celt_lcg_rand(1), 1015568748);
        assert_eq!(celt_lcg_rand(0xFFFFFFFF), 1012239698);
    }

    #[test]
    fn bitexact_cos_known_values() {
        /* Values checked by celt/tests/test_unit_mathops.c */
        assert_eq!(bitexact_cos(64), 32767);
        assert_eq!(bitexact_cos(16320), 200);
        assert_eq!(bitexact_cos(8192), 23171);
    }

    #[test]
    fn bitexact_log2tan_known_values() {
        /* Values checked by celt/tests/test_unit_mathops.c */
        assert_eq!(bitexact_log2tan(32767, 200), 15059);
        assert_eq!(bitexact_log2tan(30274, 12540), 2611);
        assert_eq!(bitexact_log2tan(23171, 23171), 0);
    }

    #[test]
    fn frac_mul16_semantics() {
        /* (16384 + (int16)a * (int16)b) >> 15 */
        assert_eq!(FRAC_MUL16(1, 1), 0);
        assert_eq!(FRAC_MUL16(16384, 16384), 8192);
        assert_eq!(FRAC_MUL16(-32768, 32767), -32767);
    }

    #[test]
    fn float_exact_values() {
        assert_eq!(celt_sqrt(4.0).to_bits(), 0x40000000);
        assert_eq!(celt_sqrt(2.0).to_bits(), 0x3fb504f3);
        assert_eq!(celt_sqrt(0.0).to_bits(), 0x00000000);
        assert_eq!(celt_rsqrt(4.0).to_bits(), 0x3f000000);
        assert_eq!(celt_rsqrt_norm(4.0).to_bits(), 0x3f000000);
        assert_eq!(celt_rcp(4.0).to_bits(), 0x3e800000);
        assert_eq!(celt_div(1.0, 2.0).to_bits(), 0x3f000000);
        assert_eq!(frac_div32(1.0, 4.0).to_bits(), 0x3e800000);
        assert_eq!(celt_exp2(0.0).to_bits(), 0x3f800000);
        assert_eq!(celt_log2(1.0).to_bits(), 0x00000000);
        assert_eq!(celt_cos_norm(0.0).to_bits(), 0x3f800000);
        assert_eq!(celt_atan_norm(0.0).to_bits(), 0x00000000);
        assert_eq!(celt_atan2p_norm(0.0, 1.0).to_bits(), 0x00000000);
        assert_eq!(celt_cos_norm2(0.0).to_bits(), 0x3f7fffff);
        assert_eq!(celt_cos_norm2(-1.0).to_bits(), 0x00000000);
        assert_eq!(celt_sin(0.0).to_bits(), 0x00000000);
    }

    #[test]
    fn maxabs() {
        let x: [f32; 5] = [0.0, -3.5, 2.25, 1.0, -0.5];
        assert_eq!(celt_maxabs16(&x, 5).to_bits(), 3.5f32.to_bits());
        assert_eq!(celt_maxabs32(&x, 5).to_bits(), 3.5f32.to_bits());
        assert_eq!(celt_maxabs_res(&x, 5).to_bits(), 3.5f32.to_bits());
        /* C: MAX32(EXTEND32(0), -EXTEND32(0)) == -0.0f */
        assert_eq!(celt_maxabs16(&x, 0).to_bits(), 0x80000000);
    }

    #[test]
    fn float_api_helpers() {
        let inp: [f32; 4] = [0.0, 0.5, -0.5, 1.0];
        let mut out: [i16; 4] = [0; 4];
        celt_float2int16_c(&inp, &mut out, 4);
        assert_eq!(out, [0, 16384, -16384, 32767]);

        let mut s: [f32; 3] = [-3.0, 0.5, 3.0];
        assert_eq!(opus_limit2_checkwithin1_c(&mut s, 3), 0);
        assert_eq!(s[0].to_bits(), (-2.0f32).to_bits());
        assert_eq!(s[1].to_bits(), 0.5f32.to_bits());
        assert_eq!(s[2].to_bits(), 2.0f32.to_bits());
        assert_eq!(opus_limit2_checkwithin1_c(&mut s, 0), 1);
    }
}

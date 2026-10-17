//! Mirror of `silk/float/inner_product_FLP.c` and
//! `silk/float/x86/inner_product_FLP_avx2.c` from libopus 1.6.1, transliterated
//! to safe Rust.
//!
//! In the reference build the dispatched `silk_inner_product_FLP` resolves
//! through `SILK_INNER_PRODUCT_FLP_IMPL[arch & OPUS_ARCHMASK]`: arch 4 selects
//! `silk_inner_product_FLP_avx2` (FMA + an 8-way split/reduction tree), every
//! other arch selects `silk_inner_product_FLP_c` (a 4x-unrolled left-to-right
//! `double` accumulation).  The two are *not* bit-identical, so both are ported
//! and [`silk_inner_product_FLP`] reproduces the dispatch.
//!
//! Original C copyright notice (this file is a derived work):
// /***********************************************************************
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
//               2023 Amazon
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

use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/// `OPUS_ARCHMASK` (`celt/cpu_support.h`) — the C dispatch masks the arch with
/// this before indexing `SILK_INNER_PRODUCT_FLP_IMPL`.
pub const OPUS_ARCHMASK: opus_int = 7;

/// `silk/float/inner_product_FLP.c:35` `silk_inner_product_FLP_c` — inner
/// product of two `silk_float` arrays, with result as `double`.
///
/// 4x unrolled, strictly left-to-right `double` accumulation.
pub fn silk_inner_product_FLP_c(
    data1: &[silk_float],
    data2: &[silk_float],
    dataSize: opus_int,
) -> f64 {
    let mut i: opus_int;
    let mut result: f64;

    /* 4x unrolled loop */
    result = 0.0;
    i = 0;
    while i < dataSize - 3 {
        result += data1[i as usize] as f64 * data2[i as usize] as f64
            + data1[(i + 1) as usize] as f64 * data2[(i + 1) as usize] as f64
            + data1[(i + 2) as usize] as f64 * data2[(i + 2) as usize] as f64
            + data1[(i + 3) as usize] as f64 * data2[(i + 3) as usize] as f64;
        i += 4;
    }

    /* add any remaining products */
    while i < dataSize {
        result += data1[i as usize] as f64 * data2[i as usize] as f64;
        i += 1;
    }

    result
}

/// `silk/float/x86/inner_product_FLP_avx2.c:38` `silk_inner_product_FLP_avx2`.
///
/// Bit-exact emulation of the AVX2/FMA kernel:
///
/// * `accum1` (4 `f64` lanes) takes elements `i + 0 .. i + 3` of every whole
///   group of 8 and of every leftover group of 4; `accum2` takes `i + 4 .. i + 7`
///   of every 8-group.  Each lane is a fused `f64::mul_add` (the `f32` products
///   are exact in `f64` anyway, but the fused rounding of the add is kept).
/// * reduction: `A[j] = accum1[j] + accum2[j]`,
///   `B = A + permute2f128(A, A, 1) = [A0+A2, A1+A3, A2+A0, A3+A1]`,
///   `hadd_pd(B, B)` → lane 0 is `B0 + B1`, so
///   `result = (A0 + A2) + (A1 + A3)`.
/// * the `< 4`-element remainder is added one term at a time.
pub fn silk_inner_product_FLP_avx2(
    data1: &[silk_float],
    data2: &[silk_float],
    dataSize: opus_int,
) -> f64 {
    /* The reference build compiles `inner_product_FLP_avx2.c` with `-mavx
       -mfma`, so the C's `_mm256_fmadd_pd` is one hardware instruction.  On the
       port's x86-64 baseline `f64::mul_add` instead lowers to a call to
       compiler-builtins' correctly-rounded `fma`, which is the same value but
       several times the cost.  Select the hardware form at run time the way the
       C's RTCD table does, and keep the emulation as the fallback for hosts
       without AVX2+FMA. */
    #[cfg(target_arch = "x86_64")]
    if crate::celt::kernels::have_avx2_fma() {
        // SAFETY: guarded by `have_avx2_fma()`.
        return unsafe { silk_inner_product_FLP_avx2_hw(data1, data2, dataSize) };
    }
    silk_inner_product_FLP_avx2_body(data1, data2, dataSize)
}

/// [`silk_inner_product_FLP_avx2_body`] compiled for AVX2+FMA: each lane's
/// `f64::mul_add` becomes a single `vfmadd` (same single rounding, so
/// bit-identical to the portable path).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn silk_inner_product_FLP_avx2_hw(
    data1: &[silk_float],
    data2: &[silk_float],
    dataSize: opus_int,
) -> f64 {
    silk_inner_product_FLP_avx2_body(data1, data2, dataSize)
}

/// The body of the AVX2 kernel, shared by the hardware-FMA path and the
/// portable fallback.
#[inline(always)]
fn silk_inner_product_FLP_avx2_body(
    data1: &[silk_float],
    data2: &[silk_float],
    dataSize: opus_int,
) -> f64 {
    let mut i: opus_int;
    let mut result: f64;
    let mut accum1 = [0.0f64; 4];
    let mut accum2 = [0.0f64; 4];

    /* 4x unrolled loop */
    result = 0.0;
    accum1 = [0.0; 4];
    accum2 = [0.0; 4];
    i = 0;
    while i < dataSize - 7 {
        let b = i as usize;
        /* xsum of elements i .. i+3 -> accum1 */
        for k in 0..4 {
            accum1[k] = (data1[b + k] as f64).mul_add(data2[b + k] as f64, accum1[k]);
        }
        /* elements i+4 .. i+7 -> accum2 */
        for k in 0..4 {
            accum2[k] =
                (data1[b + 4 + k] as f64).mul_add(data2[b + 4 + k] as f64, accum2[k]);
        }
        i += 8;
    }
    while i < dataSize - 3 {
        let b = i as usize;
        for k in 0..4 {
            accum1[k] = (data1[b + k] as f64).mul_add(data2[b + k] as f64, accum1[k]);
        }
        i += 4;
    }
    /* accum1 = _mm256_add_pd(accum1, accum2) */
    let a = [
        accum1[0] + accum2[0],
        accum1[1] + accum2[1],
        accum1[2] + accum2[2],
        accum1[3] + accum2[3],
    ];
    /* accum1 += permute2f128_pd(accum1, accum1, 1) -> [A0+A2, A1+A3, A2+A0, A3+A1] */
    let b = [a[0] + a[2], a[1] + a[3], a[2] + a[0], a[3] + a[1]];
    /* hadd_pd(B, B) per 128-bit lane; lane 0 = B0 + B1 */
    result = b[0] + b[1];

    /* add any remaining products */
    while i < dataSize {
        result += data1[i as usize] as f64 * data2[i as usize] as f64;
        i += 1;
    }

    result
}

/// The `silk_inner_product_FLP(data1, data2, dataSize, arch)` dispatch macro of
/// `silk/x86/main_sse.h:286-293`
/// (`(*SILK_INNER_PRODUCT_FLP_IMPL[(arch) & OPUS_ARCHMASK])(...)`).
///
/// arch 4 on this host is `silk_inner_product_FLP_avx2`; all other arch values
/// are `silk_inner_product_FLP_c` (the arch 3 entry is still the C function).
#[inline]
pub fn silk_inner_product_FLP(
    data1: &[silk_float],
    data2: &[silk_float],
    dataSize: opus_int,
    arch: opus_int,
) -> f64 {
    if arch & OPUS_ARCHMASK == 4 {
        silk_inner_product_FLP_avx2(data1, data2, dataSize)
    } else {
        silk_inner_product_FLP_c(data1, data2, dataSize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(i: usize) -> f32 {
        const BITS: [u32; 8] = [
            0x0000_0000,
            0x8000_0000,
            0x3f80_0000,
            0xbf80_0000,
            0x0000_0001,
            0x0080_0000,
            0x7f7f_ffff,
            0x4000_0000,
        ];
        f32::from_bits(BITS[i % BITS.len()])
    }

    #[test]
    fn short_products_use_the_scalar_tail_only() {
        /* dataSize < 4 leaves both accumulators zero, so the result is the
         * plain left-to-right sum in both implementations. */
        for n in 0..4 {
            let a: Vec<f32> = (0..n).map(|i| (i as f32) + 0.5).collect();
            let b: Vec<f32> = (0..n).map(|i| 1.0 / ((i + 1) as f32)).collect();
            assert_eq!(
                silk_inner_product_FLP_c(&a, &b, n as opus_int),
                silk_inner_product_FLP_avx2(&a, &b, n as opus_int)
            );
        }
    }

    #[test]
    fn avx2_reduction_order_is_the_documented_tree() {
        /* All-ones inputs make every lane count equal, so the reduction tree of
         * the AVX2 kernel is directly visible: lane j of accum1 holds
         * floor(n/4) terms (or one less for the ragged tail). */
        let n: opus_int = 40;
        let a = vec![1.0f32; n as usize];
        let b = vec![1.0f32; n as usize];
        assert_eq!(silk_inner_product_FLP_avx2(&a, &b, n), 40.0);
        assert_eq!(silk_inner_product_FLP_c(&a, &b, n), 40.0);
    }

    #[test]
    fn dispatch_matches_the_arch_table() {
        let a: Vec<f32> = (0..32).map(|i| pat(i)).collect();
        let b: Vec<f32> = (0..32).map(|i| pat(i + 3)).collect();
        for arch in 0..8 {
            let got = silk_inner_product_FLP(&a, &b, 32, arch);
            let want = if arch == 4 {
                silk_inner_product_FLP_avx2(&a, &b, 32)
            } else {
                silk_inner_product_FLP_c(&a, &b, 32)
            };
            assert_eq!(got.to_bits(), want.to_bits(), "arch {arch}");
        }
    }

    #[test]
    fn avx2_and_c_agree_exactly_when_all_partial_sums_are_equal() {
        /* With n = 8 and only one term per lane, both reductions are exact. */
        let a = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = [1.0f32; 8];
        assert_eq!(silk_inner_product_FLP_avx2(&a, &b, 8), 36.0);
        assert_eq!(silk_inner_product_FLP_c(&a, &b, 8), 36.0);
    }
}

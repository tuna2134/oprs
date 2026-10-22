//! Bit-exact scalar emulation of the x86 SIMD kernels the reference `libopus`
//! build executes at run time (floating-point build, `OPUS_HAVE_RTCD`,
//! `OPUS_X86_PRESUME_SSE`/`OPUS_X86_PRESUME_SSE2`, host `arch == 4`:
//! AVX2 + FMA).
//!
//! This file is a transliteration of the following C sources; the C arithmetic
//! (number of partial accumulators, lane grouping, reduction order, FMA vs
//! mul+add, loop/tail structure) is reproduced operation for operation:
//!
//! | Rust | C |
//! | --- | --- |
//! | [`xcorr_kernel`] | `celt/x86/pitch_sse.c:43` `xcorr_kernel_sse` |
//! | [`xcorr_kernel_c`] | `celt/pitch.h:65` `xcorr_kernel_c` |
//! | [`celt_inner_prod`] | `celt/x86/pitch_sse.c:108` `celt_inner_prod_sse` |
//! | [`celt_inner_prod_c`] | `celt/pitch.h:159` `celt_inner_prod_c` |
//! | [`dual_inner_prod`] | `celt/x86/pitch_sse.c:79` `dual_inner_prod_sse` |
//! | [`dual_inner_prod_c`] | `celt/pitch.h:137` `dual_inner_prod_c` |
//! | [`comb_filter_const`] | `celt/x86/pitch_sse.c:133` `comb_filter_const_sse` |
//! | [`comb_filter_const_c`] | `celt/celt.c:166` `comb_filter_const_c` |
//! | [`celt_fir`] | `celt/celt_lpc.c:146` `celt_fir_c` (float; calls the SSE `xcorr_kernel`) |
//! | [`xcorr_kernel_avx`] | `celt/x86/pitch_avx.c:39` (static) |
//! | [`celt_pitch_xcorr`] | `celt/x86/pitch_avx.c:86` `celt_pitch_xcorr_avx2` |
//! | [`celt_pitch_xcorr_c`] | `celt/pitch.c:230` `celt_pitch_xcorr_c` |
//! | [`op_pvq_search_sse2`] | `celt/x86/vq_sse2.c:44` `op_pvq_search_sse2` |
//! | [`op_pvq_search_c`] | `celt/vq.c:205` `op_pvq_search_c` |
//!
//! Notes on the configuration, verified against the pinned reference build
//! (`nm reference/lib-ref/libopus.a`):
//!
//! * `OPUS_X86_PRESUME_SSE`/`OPUS_X86_PRESUME_SSE2` are defined, so
//!   `xcorr_kernel`, `celt_inner_prod`, `dual_inner_prod`, `comb_filter_const`
//!   and `op_pvq_search` are wired *directly* to their SSE/SSE2 kernels at every
//!   `arch` value; the `arch` argument is a no-op for them.  The `*_c`
//!   functions below exist because the plain-C versions are still compiled as
//!   external symbols and the probe compares against them to document the
//!   difference; they are not reachable in the reference configuration.
//! * `celt_fir_sse4_1` is `#if defined(FIXED_POINT)`-only and is absent from
//!   the reference library (there is a `T celt_fir_c`, no `celt_fir_sse4_1`),
//!   so [`celt_fir`] is the float `celt_fir_c`, which itself calls
//!   `xcorr_kernel` — i.e. `xcorr_kernel_sse`.
//! * `celt_pitch_xcorr` is the only CELT kernel behind a real dispatch table
//!   (`PITCH_XCORR_IMPL`); at `arch == 4` it selects `celt_pitch_xcorr_avx2`.
//! * FMA is used in exactly one CELT kernel: `xcorr_kernel_avx`.  Everywhere
//!   else the C uses separate `_mm_mul_ps`/`_mm_add_ps`.  The crate is built for
//!   the x86-64 baseline (no `-mfma`, no `.cargo/config`), so LLVM cannot
//!   contract plain `a * b + c` into an FMA, and `f32::mul_add` is used exactly
//!   where the C uses a fused intrinsic.  The opt-in `fast` feature adds
//!   AVX2+FMA *variants* of some kernels (see the `[fast]` section below and
//!   `docs/progress.md`); it never changes these default bodies.
//!
//! # Slice-length requirements (the C reads past the logical end)
//!
//! The SIMD kernels load 4 (or 8) consecutive floats, so several of them read
//! up to 3 (7 for AVX2) elements past the logical end of an operand.  The Rust
//! functions never index out of bounds; instead the caller must pass a *padded*
//! slice and the functions use the exact same read pattern inside those bounds.
//! Each function documents its requirement, and the callers
//! (`pitch.rs`, `celt_lpc.rs`, `vq.rs`, `bands.rs`) must allocate the same
//! padding the C callers do (`ALLOC(x, N+3, ...)` etc.).  Required lengths are
//! checked with `debug_assert!`.
//!
//! Two functions also use the C "negative index" idiom and therefore need a
//! documented slice offset (see [`comb_filter_const`] and [`celt_fir`]).
//!
//! Copyright (c) 2014, Cisco Systems, INC  Written by XiangMingZhu WeiZhou
//! MinPeng YanWang; Copyright (c) 2007-2008 CSIRO; Copyright (c) 2007-2009
//! Xiph.Org Foundation; Copyright (c) 2007-2016 Jean-Marc Valin;
//! Copyright (c) 2023 Amazon.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are met:
//! - Redistributions of source code must retain the above copyright notice,
//!   this list of conditions and the following disclaimer.
//! - Redistributions in binary form must reproduce the above copyright notice,
//!   this list of conditions and the following disclaimer in the documentation
//!   and/or other materials provided with the distribution.
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS ``AS IS''
//! AND ANY EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.  This file is a
//! transliteration, not the original work.

use crate::celt::arch::*;
use crate::celt::celt::MAX_PVQ_N;
use crate::celt::celt_lpc::CELT_LPC_ORDER;
use crate::celt::mathops::celt_rcp;

// ---------------------------------------------------------------------------
// Lane helpers.  `[f32; 4]` models `__m128`, `[f32; 8]` models `__m256`, and
// `[i32; 4]` models `__m128i`.  The element-wise operations below perform
// exactly one IEEE-754 binary32 operation per lane, in the same order as the
// corresponding SSE/AVX intrinsics, so the results are bit-identical.
//
// On x86_64 the SSE/SSE2 kernels are transcribed with real intrinsics, so these
// helpers only back the portable `#[cfg(not(target_arch = "x86_64"))]`
// emulations and are allowed to be unused there.
// ---------------------------------------------------------------------------

/// `_mm_add_ps`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn v4_add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

/// `_mm_mul_ps`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn v4_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

/// `_mm_andnot_ps(_mm_set_ps1(-0.f), v)`: clears the sign bit of every lane.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn v4_abs(v: [f32; 4]) -> [f32; 4] {
    [
        f32::from_bits(v[0].to_bits() & 0x7fff_ffff),
        f32::from_bits(v[1].to_bits() & 0x7fff_ffff),
        f32::from_bits(v[2].to_bits() & 0x7fff_ffff),
        f32::from_bits(v[3].to_bits() & 0x7fff_ffff),
    ]
}

/// `_mm_loadu_ps(s + i)`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn v4_load(s: &[f32], i: usize) -> [f32; 4] {
    [s[i], s[i + 1], s[i + 2], s[i + 3]]
}

/// `_mm_storeu_ps(d + i, v)`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn v4_store(d: &mut [f32], i: usize, v: [f32; 4]) {
    d[i] = v[0];
    d[i + 1] = v[1];
    d[i + 2] = v[2];
    d[i + 3] = v[3];
}

/// `_mm_shuffle_ps(a, b, imm)`: `dst0=a[imm[1:0]]`, `dst1=a[imm[3:2]]`,
/// `dst2=b[imm[5:4]]`, `dst3=b[imm[7:6]]`.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn shuffle_ps(a: [f32; 4], b: [f32; 4], imm: u32) -> [f32; 4] {
    [
        a[(imm & 3) as usize],
        a[((imm >> 2) & 3) as usize],
        b[((imm >> 4) & 3) as usize],
        b[((imm >> 6) & 3) as usize],
    ]
}

/// `_mm_shuffle_epi32(a, imm)`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn shuffle_epi32(a: [i32; 4], imm: u32) -> [i32; 4] {
    [
        a[(imm & 3) as usize],
        a[((imm >> 2) & 3) as usize],
        a[((imm >> 4) & 3) as usize],
        a[((imm >> 6) & 3) as usize],
    ]
}

/// `_mm_add_epi32` (wrapping, like the C `int` add)
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn v4_add_epi32(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
        a[3].wrapping_add(b[3]),
    ]
}

#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn i16_lanes(v: i32) -> [i16; 2] {
    let b = v.to_le_bytes();
    [
        i16::from_le_bytes([b[0], b[1]]),
        i16::from_le_bytes([b[2], b[3]]),
    ]
}

#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn from_i16_lanes(l: [i16; 2]) -> i32 {
    let a = l[0].to_le_bytes();
    let b = l[1].to_le_bytes();
    i32::from_le_bytes([a[0], a[1], b[0], b[1]])
}

/// `_mm_max_epi16` (signed 16-bit lane-wise maximum; the packed-16-bit
/// "trick" in `op_pvq_search_sse2` relies on this exact operation).
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn max_epi16(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let mut o = [0i32; 4];
    let mut l = 0;
    while l < 4 {
        let x = i16_lanes(a[l]);
        let y = i16_lanes(b[l]);
        o[l] = from_i16_lanes([
            if x[0] > y[0] { x[0] } else { y[0] },
            if x[1] > y[1] { x[1] } else { y[1] },
        ]);
        l += 1;
    }
    o
}

/// `_mm_max_ps(a, b)` = `a > b ? a : b` (so a NaN in either operand yields the
/// second operand).
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn max_ps(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        if a[0] > b[0] { a[0] } else { b[0] },
        if a[1] > b[1] { a[1] } else { b[1] },
        if a[2] > b[2] { a[2] } else { b[2] },
        if a[3] > b[3] { a[3] } else { b[3] },
    ]
}

/// `_mm_cmpgt_ps(a, b)` as a 32-bit lane mask
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn cmpgt_ps_mask(a: [f32; 4], b: [f32; 4]) -> [i32; 4] {
    let m = |x: f32, y: f32| if x > y { -1i32 } else { 0i32 };
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), m(a[3], b[3])]
}

/// `_mm_cmplt_ps(a, b)` as a 32-bit lane mask
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn cmplt_ps_mask(a: [f32; 4], b: [f32; 4]) -> [i32; 4] {
    let m = |x: f32, y: f32| if x < y { -1i32 } else { 0i32 };
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), m(a[3], b[3])]
}

/// `_mm_cmpeq_ps(a, b)` as a 32-bit lane mask
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn cmpeq_ps_mask(a: [f32; 4], b: [f32; 4]) -> [i32; 4] {
    let m = |x: f32, y: f32| if x == y { -1i32 } else { 0i32 };
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), m(a[3], b[3])]
}

/// `_mm_cvttps_epi32`: truncate toward zero; out-of-range or NaN produce the
/// integer indefinite `0x80000000` (Rust's `as` would saturate instead).
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn cvttps_epi32(a: [f32; 4]) -> [i32; 4] {
    let f = |x: f32| -> i32 {
        if x >= -2147483648.0 && x < 2147483648.0 {
            x as i32
        } else {
            i32::MIN
        }
    };
    [f(a[0]), f(a[1]), f(a[2]), f(a[3])]
}

/// `_mm_cvtepi32_ps`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn cvtepi32_ps(a: [i32; 4]) -> [f32; 4] {
    [a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32]
}

/// `_mm_set1_ps`
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn splat4(x: f32) -> [f32; 4] {
    [x; 4]
}

/// `_mm256_add_ps`
#[inline(always)]
fn v8_add(a: [f32; 8], b: [f32; 8]) -> [f32; 8] {
    [
        a[0] + b[0],
        a[1] + b[1],
        a[2] + b[2],
        a[3] + b[3],
        a[4] + b[4],
        a[5] + b[5],
        a[6] + b[6],
        a[7] + b[7],
    ]
}

/// `_mm256_loadu_ps(s + i)`
#[inline(always)]
fn v8_load(s: &[f32], i: usize) -> [f32; 8] {
    [
        s[i],
        s[i + 1],
        s[i + 2],
        s[i + 3],
        s[i + 4],
        s[i + 5],
        s[i + 6],
        s[i + 7],
    ]
}

/// `_mm256_fmadd_ps(a, b, c)` = `fma(a, b, c)` with a single rounding.
#[inline(always)]
fn v8_fmadd(a: [f32; 8], b: [f32; 8], c: [f32; 8]) -> [f32; 8] {
    [
        a[0].mul_add(b[0], c[0]),
        a[1].mul_add(b[1], c[1]),
        a[2].mul_add(b[2], c[2]),
        a[3].mul_add(b[3], c[3]),
        a[4].mul_add(b[4], c[4]),
        a[5].mul_add(b[5], c[5]),
        a[6].mul_add(b[6], c[6]),
        a[7].mul_add(b[7], c[7]),
    ]
}

/// `_mm256_permute2f128_ps(a, b, imm)`.  Selector encoding per 128-bit half:
/// `0 = a.lo, 1 = a.hi, 2 = b.lo, 3 = b.hi`; the zeroing bits are unused here.
#[inline(always)]
fn permute2f128(a: [f32; 8], b: [f32; 8], imm: u32) -> [f32; 8] {
    let half = |sel: u32| -> [f32; 4] {
        match sel & 3 {
            0 => [a[0], a[1], a[2], a[3]],
            1 => [a[4], a[5], a[6], a[7]],
            2 => [b[0], b[1], b[2], b[3]],
            _ => [b[4], b[5], b[6], b[7]],
        }
    };
    let lo = half(imm & 0x0F);
    let hi = half((imm >> 4) & 0x0F);
    [lo[0], lo[1], lo[2], lo[3], hi[0], hi[1], hi[2], hi[3]]
}

/// `_mm256_hadd_ps(a, b)`, per 128-bit lane: `[a0+a1, a2+a3, b0+b1, b2+b3]`.
#[inline(always)]
fn hadd8(a: [f32; 8], b: [f32; 8]) -> [f32; 8] {
    [
        a[0] + a[1],
        a[2] + a[3],
        b[0] + b[1],
        b[2] + b[3],
        a[4] + a[5],
        a[6] + a[7],
        b[4] + b[5],
        b[6] + b[7],
    ]
}

// ---------------------------------------------------------------------------
// `_mm_rcp_ps` / `_mm_rsqrt_ps` emulation.
//
// The SSE reciprocal/square-root instructions are *not* IEEE operations: they
// return a value produced by a hardware lookup table (this is what makes
// `op_pvq_search_sse2` differ from the C `1.f/sum` / exact-ratio version).
// They cannot be called without `unsafe`, so their behaviour is reproduced
// here from the tables below.
//
// The tables were extracted from the reference host CPU (Intel Xeon E5-2690 v4,
// family 6 model 79 "Broadwell") with `_mm_rcp_ps`/`_mm_rsqrt_ps` and are the
// *only* CPU-specific data in the port.  Both instructions keep 12 significant
// mantissa bits, so each entry stores `mantissa >> 11`.
//
// **Portability caveat**: `RCPPS`/`RSQRTPS` results are microarchitecture
// specific.  These tables match the reference host; if the port ever runs on a
// different CPU family the tables (and therefore `op_pvq_search`) may need to be
// re-extracted with:
//
// ```c
// /* RCP_TAB[i] = (_mm_rcp_ps(0x3f800000|(i<<12)).bits & 0x7fffff) >> 11 */
// for (i = 0; i < 2048; i++)
//     rcp[i] = fb(_mm_cvtss_f32(_mm_rcp_ps(_mm_set1_ps(bits(0x3f800000 | (i << 12)))))) >> 11;
// /* RSQRT_TAB[idx], idx = (e&1)<<11 | (m>>12), extracted at e = 126|127 */
// for (idx = 0; idx < 4096; idx++) {
//     int e = 126 + ((idx >> 11) & 1), m = (idx & 0x7ff) << 12;
//     rsqrt[idx] = fb(_mm_cvtss_f32(_mm_rsqrt_ps(_mm_set1_ps(bits((e << 23) | m))))) >> 11;
// }
// ```
//
// The verification (compat/c/probe_kernels.c section 0) compares the emulation
// against `_mm_rcp_ps`/`_mm_rsqrt_ps` on this host value by value.
//
// The model was verified over 22.9M inputs (all 256x2048 exponent/top-mantissa
// combinations, 20M pseudo-random bit patterns, subnormals, and every special
// value) with zero mismatches:
//
// * `_mm_rcp_ps(x)`: sign preserved; `e == 0` (zero/subnormal) -> +/-inf;
//   `e == 255`: inf -> +/-0, NaN -> quieted NaN; otherwise `re = 253 - e`,
//   `re <= 0` (underflow) -> +/-0, else `sign | re<<23 | RCP_TAB[m>>12]<<11`.
// * `_mm_rsqrt_ps(x)`: `e == 0` -> sign|+inf; `e == 255`: +inf -> +0,
//   -inf -> `0xffc00000`, NaN -> quieted NaN; a negative finite input ->
//   `0xffc00000` (SSE indefinite); otherwise `idx = ((e&1)<<11)|(m>>12)`,
//   `re = 190 - ((e+1)>>1)`, result `re<<23 | RSQRT_TAB[idx]<<11`.
//
// Sanity cross-check for `RCP_TAB` (0 mismatches over all 2048 entries):
// `RCP_TAB[i] == round(2^25 / (2*i + 4097)) - 4096`.
// ---------------------------------------------------------------------------

const RCP_TAB: [u16; 2048] = [
0xffe,0xffa,0xff6,0xff2,0xfee,0xfea,0xfe6,0xfe2,0xfde,0xfda,0xfd6,0xfd2,0xfce,0xfca,0xfc6,0xfc2,
0xfbf,0xfbb,0xfb7,0xfb3,0xfaf,0xfab,0xfa7,0xfa3,0xf9f,0xf9b,0xf97,0xf93,0xf90,0xf8c,0xf88,0xf84,
0xf80,0xf7c,0xf78,0xf74,0xf71,0xf6d,0xf69,0xf65,0xf61,0xf5d,0xf59,0xf56,0xf52,0xf4e,0xf4a,0xf46,
0xf42,0xf3f,0xf3b,0xf37,0xf33,0xf2f,0xf2c,0xf28,0xf24,0xf20,0xf1c,0xf19,0xf15,0xf11,0xf0d,0xf0a,
0xf06,0xf02,0xefe,0xefb,0xef7,0xef3,0xeef,0xeec,0xee8,0xee4,0xee0,0xedd,0xed9,0xed5,0xed2,0xece,
0xeca,0xec6,0xec3,0xebf,0xebb,0xeb8,0xeb4,0xeb0,0xead,0xea9,0xea5,0xea2,0xe9e,0xe9a,0xe97,0xe93,
0xe8f,0xe8c,0xe88,0xe84,0xe81,0xe7d,0xe7a,0xe76,0xe72,0xe6f,0xe6b,0xe67,0xe64,0xe60,0xe5d,0xe59,
0xe55,0xe52,0xe4e,0xe4b,0xe47,0xe44,0xe40,0xe3c,0xe39,0xe35,0xe32,0xe2e,0xe2b,0xe27,0xe23,0xe20,
0xe1c,0xe19,0xe15,0xe12,0xe0e,0xe0b,0xe07,0xe04,0xe00,0xdfd,0xdf9,0xdf6,0xdf2,0xdef,0xdeb,0xde8,
0xde4,0xde1,0xddd,0xdda,0xdd6,0xdd3,0xdcf,0xdcc,0xdc8,0xdc5,0xdc1,0xdbe,0xdba,0xdb7,0xdb4,0xdb0,
0xdad,0xda9,0xda6,0xda2,0xd9f,0xd9b,0xd98,0xd95,0xd91,0xd8e,0xd8a,0xd87,0xd84,0xd80,0xd7d,0xd79,
0xd76,0xd73,0xd6f,0xd6c,0xd68,0xd65,0xd62,0xd5e,0xd5b,0xd58,0xd54,0xd51,0xd4e,0xd4a,0xd47,0xd44,
0xd40,0xd3d,0xd39,0xd36,0xd33,0xd2f,0xd2c,0xd29,0xd26,0xd22,0xd1f,0xd1c,0xd18,0xd15,0xd12,0xd0e,
0xd0b,0xd08,0xd04,0xd01,0xcfe,0xcfb,0xcf7,0xcf4,0xcf1,0xcee,0xcea,0xce7,0xce4,0xce0,0xcdd,0xcda,
0xcd7,0xcd3,0xcd0,0xccd,0xcca,0xcc7,0xcc3,0xcc0,0xcbd,0xcba,0xcb6,0xcb3,0xcb0,0xcad,0xcaa,0xca6,
0xca3,0xca0,0xc9d,0xc99,0xc96,0xc93,0xc90,0xc8d,0xc8a,0xc86,0xc83,0xc80,0xc7d,0xc7a,0xc77,0xc73,
0xc70,0xc6d,0xc6a,0xc67,0xc64,0xc60,0xc5d,0xc5a,0xc57,0xc54,0xc51,0xc4e,0xc4a,0xc47,0xc44,0xc41,
0xc3e,0xc3b,0xc38,0xc35,0xc32,0xc2e,0xc2b,0xc28,0xc25,0xc22,0xc1f,0xc1c,0xc19,0xc16,0xc13,0xc10,
0xc0c,0xc09,0xc06,0xc03,0xc00,0xbfd,0xbfa,0xbf7,0xbf4,0xbf1,0xbee,0xbeb,0xbe8,0xbe5,0xbe2,0xbdf,
0xbdc,0xbd9,0xbd6,0xbd3,0xbd0,0xbcd,0xbca,0xbc6,0xbc3,0xbc0,0xbbd,0xbba,0xbb7,0xbb4,0xbb1,0xbae,
0xbab,0xba8,0xba6,0xba3,0xba0,0xb9d,0xb9a,0xb97,0xb94,0xb91,0xb8e,0xb8b,0xb88,0xb85,0xb82,0xb7f,
0xb7c,0xb79,0xb76,0xb73,0xb70,0xb6d,0xb6a,0xb67,0xb64,0xb61,0xb5f,0xb5c,0xb59,0xb56,0xb53,0xb50,
0xb4d,0xb4a,0xb47,0xb44,0xb41,0xb3f,0xb3c,0xb39,0xb36,0xb33,0xb30,0xb2d,0xb2a,0xb27,0xb25,0xb22,
0xb1f,0xb1c,0xb19,0xb16,0xb13,0xb10,0xb0e,0xb0b,0xb08,0xb05,0xb02,0xaff,0xafc,0xafa,0xaf7,0xaf4,
0xaf1,0xaee,0xaeb,0xae9,0xae6,0xae3,0xae0,0xadd,0xada,0xad8,0xad5,0xad2,0xacf,0xacc,0xaca,0xac7,
0xac4,0xac1,0xabe,0xabc,0xab9,0xab6,0xab3,0xab1,0xaae,0xaab,0xaa8,0xaa5,0xaa3,0xaa0,0xa9d,0xa9a,
0xa98,0xa95,0xa92,0xa8f,0xa8d,0xa8a,0xa87,0xa84,0xa82,0xa7f,0xa7c,0xa79,0xa77,0xa74,0xa71,0xa6e,
0xa6c,0xa69,0xa66,0xa63,0xa61,0xa5e,0xa5b,0xa59,0xa56,0xa53,0xa50,0xa4e,0xa4b,0xa48,0xa46,0xa43,
0xa40,0xa3e,0xa3b,0xa38,0xa36,0xa33,0xa30,0xa2e,0xa2b,0xa28,0xa25,0xa23,0xa20,0xa1d,0xa1b,0xa18,
0xa15,0xa13,0xa10,0xa0e,0xa0b,0xa08,0xa06,0xa03,0xa00,0x9fe,0x9fb,0x9f8,0x9f6,0x9f3,0x9f0,0x9ee,
0x9eb,0x9e9,0x9e6,0x9e3,0x9e1,0x9de,0x9dc,0x9d9,0x9d6,0x9d4,0x9d1,0x9cf,0x9cc,0x9c9,0x9c7,0x9c4,
0x9c2,0x9bf,0x9bc,0x9ba,0x9b7,0x9b5,0x9b2,0x9af,0x9ad,0x9aa,0x9a8,0x9a5,0x9a3,0x9a0,0x99d,0x99b,
0x998,0x996,0x993,0x991,0x98e,0x98c,0x989,0x986,0x984,0x981,0x97f,0x97c,0x97a,0x977,0x975,0x972,
0x970,0x96d,0x96b,0x968,0x966,0x963,0x961,0x95e,0x95b,0x959,0x956,0x954,0x951,0x94f,0x94c,0x94a,
0x947,0x945,0x942,0x940,0x93d,0x93b,0x939,0x936,0x934,0x931,0x92f,0x92c,0x92a,0x927,0x925,0x922,
0x920,0x91d,0x91b,0x918,0x916,0x913,0x911,0x90f,0x90c,0x90a,0x907,0x905,0x902,0x900,0x8fd,0x8fb,
0x8f9,0x8f6,0x8f4,0x8f1,0x8ef,0x8ec,0x8ea,0x8e8,0x8e5,0x8e3,0x8e0,0x8de,0x8db,0x8d9,0x8d7,0x8d4,
0x8d2,0x8cf,0x8cd,0x8cb,0x8c8,0x8c6,0x8c3,0x8c1,0x8bf,0x8bc,0x8ba,0x8b7,0x8b5,0x8b3,0x8b0,0x8ae,
0x8ac,0x8a9,0x8a7,0x8a4,0x8a2,0x8a0,0x89d,0x89b,0x899,0x896,0x894,0x891,0x88f,0x88d,0x88a,0x888,
0x886,0x883,0x881,0x87f,0x87c,0x87a,0x878,0x875,0x873,0x871,0x86e,0x86c,0x86a,0x867,0x865,0x863,
0x860,0x85e,0x85c,0x859,0x857,0x855,0x852,0x850,0x84e,0x84c,0x849,0x847,0x845,0x842,0x840,0x83e,
0x83b,0x839,0x837,0x835,0x832,0x830,0x82e,0x82b,0x829,0x827,0x825,0x822,0x820,0x81e,0x81b,0x819,
0x817,0x815,0x812,0x810,0x80e,0x80c,0x809,0x807,0x805,0x803,0x800,0x7fe,0x7fc,0x7fa,0x7f7,0x7f5,
0x7f3,0x7f1,0x7ee,0x7ec,0x7ea,0x7e8,0x7e5,0x7e3,0x7e1,0x7df,0x7dd,0x7da,0x7d8,0x7d6,0x7d4,0x7d1,
0x7cf,0x7cd,0x7cb,0x7c9,0x7c6,0x7c4,0x7c2,0x7c0,0x7be,0x7bb,0x7b9,0x7b7,0x7b5,0x7b3,0x7b0,0x7ae,
0x7ac,0x7aa,0x7a8,0x7a5,0x7a3,0x7a1,0x79f,0x79d,0x79b,0x798,0x796,0x794,0x792,0x790,0x78e,0x78b,
0x789,0x787,0x785,0x783,0x781,0x77e,0x77c,0x77a,0x778,0x776,0x774,0x772,0x76f,0x76d,0x76b,0x769,
0x767,0x765,0x763,0x760,0x75e,0x75c,0x75a,0x758,0x756,0x754,0x751,0x74f,0x74d,0x74b,0x749,0x747,
0x745,0x743,0x741,0x73e,0x73c,0x73a,0x738,0x736,0x734,0x732,0x730,0x72e,0x72b,0x729,0x727,0x725,
0x723,0x721,0x71f,0x71d,0x71b,0x719,0x717,0x715,0x712,0x710,0x70e,0x70c,0x70a,0x708,0x706,0x704,
0x702,0x700,0x6fe,0x6fc,0x6fa,0x6f8,0x6f5,0x6f3,0x6f1,0x6ef,0x6ed,0x6eb,0x6e9,0x6e7,0x6e5,0x6e3,
0x6e1,0x6df,0x6dd,0x6db,0x6d9,0x6d7,0x6d5,0x6d3,0x6d1,0x6cf,0x6cd,0x6cb,0x6c9,0x6c6,0x6c4,0x6c2,
0x6c0,0x6be,0x6bc,0x6ba,0x6b8,0x6b6,0x6b4,0x6b2,0x6b0,0x6ae,0x6ac,0x6aa,0x6a8,0x6a6,0x6a4,0x6a2,
0x6a0,0x69e,0x69c,0x69a,0x698,0x696,0x694,0x692,0x690,0x68e,0x68c,0x68a,0x688,0x686,0x684,0x682,
0x680,0x67e,0x67c,0x67a,0x679,0x677,0x675,0x673,0x671,0x66f,0x66d,0x66b,0x669,0x667,0x665,0x663,
0x661,0x65f,0x65d,0x65b,0x659,0x657,0x655,0x653,0x651,0x64f,0x64d,0x64c,0x64a,0x648,0x646,0x644,
0x642,0x640,0x63e,0x63c,0x63a,0x638,0x636,0x634,0x632,0x630,0x62f,0x62d,0x62b,0x629,0x627,0x625,
0x623,0x621,0x61f,0x61d,0x61b,0x619,0x618,0x616,0x614,0x612,0x610,0x60e,0x60c,0x60a,0x608,0x606,
0x605,0x603,0x601,0x5ff,0x5fd,0x5fb,0x5f9,0x5f7,0x5f5,0x5f4,0x5f2,0x5f0,0x5ee,0x5ec,0x5ea,0x5e8,
0x5e6,0x5e5,0x5e3,0x5e1,0x5df,0x5dd,0x5db,0x5d9,0x5d7,0x5d6,0x5d4,0x5d2,0x5d0,0x5ce,0x5cc,0x5ca,
0x5c9,0x5c7,0x5c5,0x5c3,0x5c1,0x5bf,0x5be,0x5bc,0x5ba,0x5b8,0x5b6,0x5b4,0x5b2,0x5b1,0x5af,0x5ad,
0x5ab,0x5a9,0x5a7,0x5a6,0x5a4,0x5a2,0x5a0,0x59e,0x59c,0x59b,0x599,0x597,0x595,0x593,0x592,0x590,
0x58e,0x58c,0x58a,0x588,0x587,0x585,0x583,0x581,0x57f,0x57e,0x57c,0x57a,0x578,0x576,0x575,0x573,
0x571,0x56f,0x56d,0x56c,0x56a,0x568,0x566,0x564,0x563,0x561,0x55f,0x55d,0x55c,0x55a,0x558,0x556,
0x554,0x553,0x551,0x54f,0x54d,0x54c,0x54a,0x548,0x546,0x544,0x543,0x541,0x53f,0x53d,0x53c,0x53a,
0x538,0x536,0x535,0x533,0x531,0x52f,0x52e,0x52c,0x52a,0x528,0x527,0x525,0x523,0x521,0x520,0x51e,
0x51c,0x51a,0x519,0x517,0x515,0x513,0x512,0x510,0x50e,0x50d,0x50b,0x509,0x507,0x506,0x504,0x502,
0x500,0x4ff,0x4fd,0x4fb,0x4fa,0x4f8,0x4f6,0x4f4,0x4f3,0x4f1,0x4ef,0x4ee,0x4ec,0x4ea,0x4e8,0x4e7,
0x4e5,0x4e3,0x4e2,0x4e0,0x4de,0x4dd,0x4db,0x4d9,0x4d7,0x4d6,0x4d4,0x4d2,0x4d1,0x4cf,0x4cd,0x4cc,
0x4ca,0x4c8,0x4c7,0x4c5,0x4c3,0x4c1,0x4c0,0x4be,0x4bc,0x4bb,0x4b9,0x4b7,0x4b6,0x4b4,0x4b2,0x4b1,
0x4af,0x4ad,0x4ac,0x4aa,0x4a8,0x4a7,0x4a5,0x4a3,0x4a2,0x4a0,0x49e,0x49d,0x49b,0x499,0x498,0x496,
0x494,0x493,0x491,0x48f,0x48e,0x48c,0x48a,0x489,0x487,0x486,0x484,0x482,0x481,0x47f,0x47d,0x47c,
0x47a,0x478,0x477,0x475,0x474,0x472,0x470,0x46f,0x46d,0x46b,0x46a,0x468,0x466,0x465,0x463,0x462,
0x460,0x45e,0x45d,0x45b,0x45a,0x458,0x456,0x455,0x453,0x451,0x450,0x44e,0x44d,0x44b,0x449,0x448,
0x446,0x445,0x443,0x441,0x440,0x43e,0x43d,0x43b,0x439,0x438,0x436,0x435,0x433,0x431,0x430,0x42e,
0x42d,0x42b,0x429,0x428,0x426,0x425,0x423,0x421,0x420,0x41e,0x41d,0x41b,0x41a,0x418,0x416,0x415,
0x413,0x412,0x410,0x40f,0x40d,0x40b,0x40a,0x408,0x407,0x405,0x404,0x402,0x400,0x3ff,0x3fd,0x3fc,
0x3fa,0x3f9,0x3f7,0x3f6,0x3f4,0x3f2,0x3f1,0x3ef,0x3ee,0x3ec,0x3eb,0x3e9,0x3e8,0x3e6,0x3e4,0x3e3,
0x3e1,0x3e0,0x3de,0x3dd,0x3db,0x3da,0x3d8,0x3d7,0x3d5,0x3d4,0x3d2,0x3d0,0x3cf,0x3cd,0x3cc,0x3ca,
0x3c9,0x3c7,0x3c6,0x3c4,0x3c3,0x3c1,0x3c0,0x3be,0x3bd,0x3bb,0x3ba,0x3b8,0x3b7,0x3b5,0x3b4,0x3b2,
0x3b0,0x3af,0x3ad,0x3ac,0x3aa,0x3a9,0x3a7,0x3a6,0x3a4,0x3a3,0x3a1,0x3a0,0x39e,0x39d,0x39b,0x39a,
0x398,0x397,0x395,0x394,0x392,0x391,0x38f,0x38e,0x38c,0x38b,0x389,0x388,0x386,0x385,0x383,0x382,
0x380,0x37f,0x37e,0x37c,0x37b,0x379,0x378,0x376,0x375,0x373,0x372,0x370,0x36f,0x36d,0x36c,0x36a,
0x369,0x367,0x366,0x364,0x363,0x361,0x360,0x35f,0x35d,0x35c,0x35a,0x359,0x357,0x356,0x354,0x353,
0x351,0x350,0x34e,0x34d,0x34c,0x34a,0x349,0x347,0x346,0x344,0x343,0x341,0x340,0x33f,0x33d,0x33c,
0x33a,0x339,0x337,0x336,0x334,0x333,0x332,0x330,0x32f,0x32d,0x32c,0x32a,0x329,0x327,0x326,0x325,
0x323,0x322,0x320,0x31f,0x31d,0x31c,0x31b,0x319,0x318,0x316,0x315,0x313,0x312,0x311,0x30f,0x30e,
0x30c,0x30b,0x30a,0x308,0x307,0x305,0x304,0x302,0x301,0x300,0x2fe,0x2fd,0x2fb,0x2fa,0x2f9,0x2f7,
0x2f6,0x2f4,0x2f3,0x2f2,0x2f0,0x2ef,0x2ed,0x2ec,0x2eb,0x2e9,0x2e8,0x2e6,0x2e5,0x2e4,0x2e2,0x2e1,
0x2df,0x2de,0x2dd,0x2db,0x2da,0x2d9,0x2d7,0x2d6,0x2d4,0x2d3,0x2d2,0x2d0,0x2cf,0x2cd,0x2cc,0x2cb,
0x2c9,0x2c8,0x2c7,0x2c5,0x2c4,0x2c2,0x2c1,0x2c0,0x2be,0x2bd,0x2bc,0x2ba,0x2b9,0x2b7,0x2b6,0x2b5,
0x2b3,0x2b2,0x2b1,0x2af,0x2ae,0x2ad,0x2ab,0x2aa,0x2a8,0x2a7,0x2a6,0x2a4,0x2a3,0x2a2,0x2a0,0x29f,
0x29e,0x29c,0x29b,0x29a,0x298,0x297,0x295,0x294,0x293,0x291,0x290,0x28f,0x28d,0x28c,0x28b,0x289,
0x288,0x287,0x285,0x284,0x283,0x281,0x280,0x27f,0x27d,0x27c,0x27b,0x279,0x278,0x277,0x275,0x274,
0x273,0x271,0x270,0x26f,0x26d,0x26c,0x26b,0x269,0x268,0x267,0x265,0x264,0x263,0x261,0x260,0x25f,
0x25d,0x25c,0x25b,0x25a,0x258,0x257,0x256,0x254,0x253,0x252,0x250,0x24f,0x24e,0x24c,0x24b,0x24a,
0x248,0x247,0x246,0x245,0x243,0x242,0x241,0x23f,0x23e,0x23d,0x23b,0x23a,0x239,0x238,0x236,0x235,
0x234,0x232,0x231,0x230,0x22f,0x22d,0x22c,0x22b,0x229,0x228,0x227,0x225,0x224,0x223,0x222,0x220,
0x21f,0x21e,0x21d,0x21b,0x21a,0x219,0x217,0x216,0x215,0x214,0x212,0x211,0x210,0x20e,0x20d,0x20c,
0x20b,0x209,0x208,0x207,0x206,0x204,0x203,0x202,0x200,0x1ff,0x1fe,0x1fd,0x1fb,0x1fa,0x1f9,0x1f8,
0x1f6,0x1f5,0x1f4,0x1f3,0x1f1,0x1f0,0x1ef,0x1ee,0x1ec,0x1eb,0x1ea,0x1e9,0x1e7,0x1e6,0x1e5,0x1e4,
0x1e2,0x1e1,0x1e0,0x1df,0x1dd,0x1dc,0x1db,0x1da,0x1d8,0x1d7,0x1d6,0x1d5,0x1d3,0x1d2,0x1d1,0x1d0,
0x1ce,0x1cd,0x1cc,0x1cb,0x1c9,0x1c8,0x1c7,0x1c6,0x1c5,0x1c3,0x1c2,0x1c1,0x1c0,0x1be,0x1bd,0x1bc,
0x1bb,0x1b9,0x1b8,0x1b7,0x1b6,0x1b5,0x1b3,0x1b2,0x1b1,0x1b0,0x1ae,0x1ad,0x1ac,0x1ab,0x1aa,0x1a8,
0x1a7,0x1a6,0x1a5,0x1a3,0x1a2,0x1a1,0x1a0,0x19f,0x19d,0x19c,0x19b,0x19a,0x199,0x197,0x196,0x195,
0x194,0x193,0x191,0x190,0x18f,0x18e,0x18c,0x18b,0x18a,0x189,0x188,0x186,0x185,0x184,0x183,0x182,
0x180,0x17f,0x17e,0x17d,0x17c,0x17b,0x179,0x178,0x177,0x176,0x175,0x173,0x172,0x171,0x170,0x16f,
0x16d,0x16c,0x16b,0x16a,0x169,0x168,0x166,0x165,0x164,0x163,0x162,0x160,0x15f,0x15e,0x15d,0x15c,
0x15b,0x159,0x158,0x157,0x156,0x155,0x153,0x152,0x151,0x150,0x14f,0x14e,0x14c,0x14b,0x14a,0x149,
0x148,0x147,0x145,0x144,0x143,0x142,0x141,0x140,0x13e,0x13d,0x13c,0x13b,0x13a,0x139,0x138,0x136,
0x135,0x134,0x133,0x132,0x131,0x12f,0x12e,0x12d,0x12c,0x12b,0x12a,0x129,0x127,0x126,0x125,0x124,
0x123,0x122,0x120,0x11f,0x11e,0x11d,0x11c,0x11b,0x11a,0x118,0x117,0x116,0x115,0x114,0x113,0x112,
0x110,0x10f,0x10e,0x10d,0x10c,0x10b,0x10a,0x109,0x107,0x106,0x105,0x104,0x103,0x102,0x101,0x100,
0x0fe,0x0fd,0x0fc,0x0fb,0x0fa,0x0f9,0x0f8,0x0f6,0x0f5,0x0f4,0x0f3,0x0f2,0x0f1,0x0f0,0x0ef,0x0ee,
0x0ec,0x0eb,0x0ea,0x0e9,0x0e8,0x0e7,0x0e6,0x0e5,0x0e3,0x0e2,0x0e1,0x0e0,0x0df,0x0de,0x0dd,0x0dc,
0x0db,0x0d9,0x0d8,0x0d7,0x0d6,0x0d5,0x0d4,0x0d3,0x0d2,0x0d1,0x0d0,0x0ce,0x0cd,0x0cc,0x0cb,0x0ca,
0x0c9,0x0c8,0x0c7,0x0c6,0x0c4,0x0c3,0x0c2,0x0c1,0x0c0,0x0bf,0x0be,0x0bd,0x0bc,0x0bb,0x0ba,0x0b8,
0x0b7,0x0b6,0x0b5,0x0b4,0x0b3,0x0b2,0x0b1,0x0b0,0x0af,0x0ae,0x0ac,0x0ab,0x0aa,0x0a9,0x0a8,0x0a7,
0x0a6,0x0a5,0x0a4,0x0a3,0x0a2,0x0a1,0x09f,0x09e,0x09d,0x09c,0x09b,0x09a,0x099,0x098,0x097,0x096,
0x095,0x094,0x093,0x091,0x090,0x08f,0x08e,0x08d,0x08c,0x08b,0x08a,0x089,0x088,0x087,0x086,0x085,
0x084,0x083,0x081,0x080,0x07f,0x07e,0x07d,0x07c,0x07b,0x07a,0x079,0x078,0x077,0x076,0x075,0x074,
0x073,0x072,0x071,0x06f,0x06e,0x06d,0x06c,0x06b,0x06a,0x069,0x068,0x067,0x066,0x065,0x064,0x063,
0x062,0x061,0x060,0x05f,0x05e,0x05d,0x05b,0x05a,0x059,0x058,0x057,0x056,0x055,0x054,0x053,0x052,
0x051,0x050,0x04f,0x04e,0x04d,0x04c,0x04b,0x04a,0x049,0x048,0x047,0x046,0x045,0x044,0x043,0x042,
0x040,0x03f,0x03e,0x03d,0x03c,0x03b,0x03a,0x039,0x038,0x037,0x036,0x035,0x034,0x033,0x032,0x031,
0x030,0x02f,0x02e,0x02d,0x02c,0x02b,0x02a,0x029,0x028,0x027,0x026,0x025,0x024,0x023,0x022,0x021,
0x020,0x01f,0x01e,0x01d,0x01c,0x01b,0x01a,0x019,0x018,0x017,0x016,0x015,0x014,0x013,0x012,0x011,
0x010,0x00f,0x00e,0x00d,0x00c,0x00b,0x00a,0x009,0x008,0x007,0x006,0x005,0x004,0x003,0x002,0x001,
];
const RSQRT_TAB: [u16; 4096] = [
0x69f,0x69f,0x69c,0x69c,0x69a,0x69a,0x697,0x697,0x694,0x694,0x691,0x691,0x68e,0x68e,0x68c,0x68c,
0x689,0x689,0x686,0x686,0x683,0x683,0x680,0x680,0x67e,0x67e,0x67b,0x67b,0x678,0x678,0x675,0x675,
0x673,0x673,0x670,0x670,0x66d,0x66d,0x66a,0x66a,0x667,0x667,0x665,0x665,0x662,0x662,0x65f,0x65f,
0x65d,0x65d,0x65a,0x65a,0x657,0x657,0x654,0x654,0x652,0x652,0x64f,0x64f,0x64c,0x64c,0x64a,0x64a,
0x647,0x647,0x644,0x644,0x641,0x641,0x63f,0x63f,0x63c,0x63c,0x639,0x639,0x637,0x637,0x634,0x634,
0x631,0x631,0x62f,0x62f,0x62c,0x62c,0x629,0x629,0x627,0x627,0x624,0x624,0x621,0x621,0x61f,0x61f,
0x61c,0x61c,0x619,0x619,0x617,0x617,0x614,0x614,0x612,0x612,0x60f,0x60f,0x60c,0x60c,0x60a,0x60a,
0x607,0x607,0x605,0x605,0x602,0x602,0x5ff,0x5ff,0x5fd,0x5fd,0x5fa,0x5fa,0x5f8,0x5f8,0x5f5,0x5f5,
0x5f2,0x5f2,0x5f0,0x5f0,0x5ed,0x5ed,0x5eb,0x5eb,0x5e8,0x5e8,0x5e6,0x5e6,0x5e3,0x5e3,0x5e0,0x5e0,
0x5de,0x5de,0x5db,0x5db,0x5d9,0x5d9,0x5d6,0x5d6,0x5d4,0x5d4,0x5d1,0x5d1,0x5cf,0x5cf,0x5cc,0x5cc,
0x5ca,0x5ca,0x5c7,0x5c7,0x5c4,0x5c4,0x5c2,0x5c2,0x5bf,0x5bf,0x5bd,0x5bd,0x5ba,0x5ba,0x5b8,0x5b8,
0x5b5,0x5b5,0x5b3,0x5b3,0x5b0,0x5b0,0x5ae,0x5ae,0x5ab,0x5ab,0x5a9,0x5a9,0x5a7,0x5a7,0x5a4,0x5a4,
0x5a2,0x5a2,0x59f,0x59f,0x59d,0x59d,0x59a,0x59a,0x598,0x598,0x595,0x595,0x593,0x593,0x590,0x590,
0x58e,0x58e,0x58b,0x58b,0x589,0x589,0x587,0x587,0x584,0x584,0x582,0x582,0x57f,0x57f,0x57d,0x57d,
0x57a,0x57a,0x578,0x578,0x576,0x576,0x573,0x573,0x571,0x571,0x56e,0x56e,0x56c,0x56c,0x56a,0x56a,
0x567,0x567,0x565,0x565,0x562,0x562,0x560,0x560,0x55e,0x55e,0x55b,0x55b,0x559,0x559,0x557,0x557,
0x554,0x554,0x552,0x552,0x54f,0x54f,0x54d,0x54d,0x54b,0x54b,0x548,0x548,0x546,0x546,0x544,0x544,
0x541,0x541,0x53f,0x53f,0x53d,0x53d,0x53a,0x53a,0x538,0x538,0x536,0x536,0x533,0x533,0x531,0x531,
0x52f,0x52f,0x52c,0x52c,0x52a,0x52a,0x528,0x528,0x525,0x525,0x523,0x523,0x521,0x521,0x51e,0x51e,
0x51c,0x51c,0x51a,0x51a,0x518,0x518,0x515,0x515,0x513,0x513,0x511,0x511,0x50e,0x50e,0x50c,0x50c,
0x50a,0x50a,0x508,0x508,0x505,0x505,0x503,0x503,0x501,0x501,0x4ff,0x4ff,0x4fc,0x4fc,0x4fa,0x4fa,
0x4f8,0x4f8,0x4f6,0x4f6,0x4f3,0x4f3,0x4f1,0x4f1,0x4ef,0x4ef,0x4ed,0x4ed,0x4ea,0x4ea,0x4e8,0x4e8,
0x4e6,0x4e6,0x4e4,0x4e4,0x4e1,0x4e1,0x4df,0x4df,0x4dd,0x4dd,0x4db,0x4db,0x4d9,0x4d9,0x4d6,0x4d6,
0x4d4,0x4d4,0x4d2,0x4d2,0x4d0,0x4d0,0x4ce,0x4ce,0x4cb,0x4cb,0x4c9,0x4c9,0x4c7,0x4c7,0x4c5,0x4c5,
0x4c3,0x4c3,0x4c0,0x4c0,0x4be,0x4be,0x4bc,0x4bc,0x4ba,0x4ba,0x4b8,0x4b8,0x4b6,0x4b6,0x4b3,0x4b3,
0x4b1,0x4b1,0x4af,0x4af,0x4ad,0x4ad,0x4ab,0x4ab,0x4a9,0x4a9,0x4a6,0x4a6,0x4a4,0x4a4,0x4a2,0x4a2,
0x4a0,0x4a0,0x49e,0x49e,0x49c,0x49c,0x49a,0x49a,0x497,0x497,0x495,0x495,0x493,0x493,0x491,0x491,
0x48f,0x48f,0x48d,0x48d,0x48b,0x48b,0x489,0x489,0x486,0x486,0x484,0x484,0x482,0x482,0x480,0x480,
0x47e,0x47e,0x47c,0x47c,0x47a,0x47a,0x478,0x478,0x476,0x476,0x474,0x474,0x471,0x471,0x46f,0x46f,
0x46d,0x46d,0x46b,0x46b,0x469,0x469,0x467,0x467,0x465,0x465,0x463,0x463,0x461,0x461,0x45f,0x45f,
0x45d,0x45d,0x45b,0x45b,0x459,0x459,0x457,0x457,0x455,0x455,0x452,0x452,0x450,0x450,0x44e,0x44e,
0x44c,0x44c,0x44a,0x44a,0x448,0x448,0x446,0x446,0x444,0x444,0x442,0x442,0x440,0x440,0x43e,0x43e,
0x43c,0x43c,0x43a,0x43a,0x438,0x438,0x436,0x436,0x434,0x434,0x432,0x432,0x430,0x430,0x42e,0x42e,
0x42c,0x42c,0x42a,0x42a,0x428,0x428,0x426,0x426,0x424,0x424,0x422,0x422,0x420,0x420,0x41e,0x41e,
0x41c,0x41c,0x41a,0x41a,0x418,0x418,0x416,0x416,0x414,0x414,0x412,0x412,0x410,0x410,0x40e,0x40e,
0x40c,0x40c,0x40a,0x40a,0x408,0x408,0x406,0x406,0x404,0x404,0x402,0x402,0x400,0x400,0x3fe,0x3fe,
0x3fd,0x3fd,0x3fb,0x3fb,0x3f9,0x3f9,0x3f7,0x3f7,0x3f5,0x3f5,0x3f3,0x3f3,0x3f1,0x3f1,0x3ef,0x3ef,
0x3ed,0x3ed,0x3eb,0x3eb,0x3e9,0x3e9,0x3e7,0x3e7,0x3e5,0x3e5,0x3e3,0x3e3,0x3e1,0x3e1,0x3e0,0x3e0,
0x3de,0x3de,0x3dc,0x3dc,0x3da,0x3da,0x3d8,0x3d8,0x3d6,0x3d6,0x3d4,0x3d4,0x3d2,0x3d2,0x3d0,0x3d0,
0x3ce,0x3ce,0x3cc,0x3cc,0x3cb,0x3cb,0x3c9,0x3c9,0x3c7,0x3c7,0x3c5,0x3c5,0x3c3,0x3c3,0x3c1,0x3c1,
0x3bf,0x3bf,0x3bd,0x3bd,0x3bc,0x3bc,0x3ba,0x3ba,0x3b8,0x3b8,0x3b6,0x3b6,0x3b4,0x3b4,0x3b2,0x3b2,
0x3b0,0x3b0,0x3ae,0x3ae,0x3ad,0x3ad,0x3ab,0x3ab,0x3a9,0x3a9,0x3a7,0x3a7,0x3a5,0x3a5,0x3a3,0x3a3,
0x3a1,0x3a1,0x3a0,0x3a0,0x39e,0x39e,0x39c,0x39c,0x39a,0x39a,0x398,0x398,0x396,0x396,0x395,0x395,
0x393,0x393,0x391,0x391,0x38f,0x38f,0x38d,0x38d,0x38b,0x38b,0x38a,0x38a,0x388,0x388,0x386,0x386,
0x384,0x384,0x382,0x382,0x381,0x381,0x37f,0x37f,0x37d,0x37d,0x37b,0x37b,0x379,0x379,0x378,0x378,
0x376,0x376,0x374,0x374,0x372,0x372,0x370,0x370,0x36f,0x36f,0x36d,0x36d,0x36b,0x36b,0x369,0x369,
0x367,0x367,0x366,0x366,0x364,0x364,0x362,0x362,0x360,0x360,0x35e,0x35e,0x35d,0x35d,0x35b,0x35b,
0x359,0x359,0x357,0x357,0x356,0x356,0x354,0x354,0x352,0x352,0x350,0x350,0x34f,0x34f,0x34d,0x34d,
0x34b,0x34b,0x349,0x349,0x348,0x348,0x346,0x346,0x344,0x344,0x342,0x342,0x341,0x341,0x33f,0x33f,
0x33d,0x33d,0x33b,0x33b,0x33a,0x33a,0x338,0x338,0x336,0x336,0x334,0x334,0x333,0x333,0x331,0x331,
0x32f,0x32f,0x32e,0x32e,0x32c,0x32c,0x32a,0x32a,0x328,0x328,0x327,0x327,0x325,0x325,0x323,0x323,
0x322,0x322,0x320,0x320,0x31e,0x31e,0x31c,0x31c,0x31b,0x31b,0x319,0x319,0x317,0x317,0x316,0x316,
0x314,0x314,0x312,0x312,0x311,0x311,0x30f,0x30f,0x30d,0x30d,0x30b,0x30b,0x30a,0x30a,0x308,0x308,
0x306,0x306,0x305,0x305,0x303,0x303,0x301,0x301,0x300,0x300,0x2fe,0x2fe,0x2fc,0x2fc,0x2fb,0x2fb,
0x2f9,0x2f9,0x2f7,0x2f7,0x2f6,0x2f6,0x2f4,0x2f4,0x2f2,0x2f2,0x2f1,0x2f1,0x2ef,0x2ef,0x2ed,0x2ed,
0x2ec,0x2ec,0x2ea,0x2ea,0x2e8,0x2e8,0x2e7,0x2e7,0x2e5,0x2e5,0x2e3,0x2e3,0x2e2,0x2e2,0x2e0,0x2e0,
0x2df,0x2df,0x2dd,0x2dd,0x2db,0x2db,0x2da,0x2da,0x2d8,0x2d8,0x2d6,0x2d6,0x2d5,0x2d5,0x2d3,0x2d3,
0x2d1,0x2d1,0x2d0,0x2d0,0x2ce,0x2ce,0x2cd,0x2cd,0x2cb,0x2cb,0x2c9,0x2c9,0x2c8,0x2c8,0x2c6,0x2c6,
0x2c5,0x2c5,0x2c3,0x2c3,0x2c1,0x2c1,0x2c0,0x2c0,0x2be,0x2be,0x2bc,0x2bc,0x2bb,0x2bb,0x2b9,0x2b9,
0x2b8,0x2b8,0x2b6,0x2b6,0x2b4,0x2b4,0x2b3,0x2b3,0x2b1,0x2b1,0x2b0,0x2b0,0x2ae,0x2ae,0x2ac,0x2ac,
0x2ab,0x2ab,0x2a9,0x2a9,0x2a8,0x2a8,0x2a6,0x2a6,0x2a5,0x2a5,0x2a3,0x2a3,0x2a1,0x2a1,0x2a0,0x2a0,
0x29e,0x29e,0x29d,0x29d,0x29b,0x29b,0x29a,0x29a,0x298,0x298,0x296,0x296,0x295,0x295,0x293,0x293,
0x292,0x292,0x290,0x290,0x28f,0x28f,0x28d,0x28d,0x28b,0x28b,0x28a,0x28a,0x288,0x288,0x287,0x287,
0x285,0x285,0x284,0x284,0x282,0x282,0x281,0x281,0x27f,0x27f,0x27e,0x27e,0x27c,0x27c,0x27a,0x27a,
0x279,0x279,0x277,0x277,0x276,0x276,0x274,0x274,0x273,0x273,0x271,0x271,0x270,0x270,0x26e,0x26e,
0x26d,0x26d,0x26b,0x26b,0x26a,0x26a,0x268,0x268,0x267,0x267,0x265,0x265,0x263,0x263,0x262,0x262,
0x260,0x260,0x25f,0x25f,0x25d,0x25d,0x25c,0x25c,0x25a,0x25a,0x259,0x259,0x257,0x257,0x256,0x256,
0x254,0x254,0x253,0x253,0x251,0x251,0x250,0x250,0x24e,0x24e,0x24d,0x24d,0x24b,0x24b,0x24a,0x24a,
0x248,0x248,0x247,0x247,0x245,0x245,0x244,0x244,0x242,0x242,0x241,0x241,0x23f,0x23f,0x23e,0x23e,
0x23d,0x23d,0x23b,0x23b,0x23a,0x23a,0x238,0x238,0x237,0x237,0x235,0x235,0x234,0x234,0x232,0x232,
0x231,0x231,0x22f,0x22f,0x22e,0x22e,0x22c,0x22c,0x22b,0x22b,0x229,0x229,0x228,0x228,0x226,0x226,
0x225,0x225,0x224,0x224,0x222,0x222,0x221,0x221,0x21f,0x21f,0x21e,0x21e,0x21c,0x21c,0x21b,0x21b,
0x219,0x219,0x218,0x218,0x216,0x216,0x215,0x215,0x214,0x214,0x212,0x212,0x211,0x211,0x20f,0x20f,
0x20e,0x20e,0x20c,0x20c,0x20b,0x20b,0x20a,0x20a,0x208,0x208,0x207,0x207,0x205,0x205,0x204,0x204,
0x202,0x202,0x201,0x201,0x200,0x200,0x1fe,0x1fe,0x1fd,0x1fd,0x1fb,0x1fb,0x1fa,0x1fa,0x1f8,0x1f8,
0x1f7,0x1f7,0x1f6,0x1f6,0x1f4,0x1f4,0x1f3,0x1f3,0x1f1,0x1f1,0x1f0,0x1f0,0x1ef,0x1ef,0x1ed,0x1ed,
0x1ec,0x1ec,0x1ea,0x1ea,0x1e9,0x1e9,0x1e8,0x1e8,0x1e6,0x1e6,0x1e5,0x1e5,0x1e3,0x1e3,0x1e2,0x1e2,
0x1e1,0x1e1,0x1df,0x1df,0x1de,0x1de,0x1dc,0x1dc,0x1db,0x1db,0x1da,0x1da,0x1d8,0x1d8,0x1d7,0x1d7,
0x1d5,0x1d5,0x1d4,0x1d4,0x1d3,0x1d3,0x1d1,0x1d1,0x1d0,0x1d0,0x1cf,0x1cf,0x1cd,0x1cd,0x1cc,0x1cc,
0x1ca,0x1ca,0x1c9,0x1c9,0x1c8,0x1c8,0x1c6,0x1c6,0x1c5,0x1c5,0x1c4,0x1c4,0x1c2,0x1c2,0x1c1,0x1c1,
0x1bf,0x1bf,0x1be,0x1be,0x1bd,0x1bd,0x1bb,0x1bb,0x1ba,0x1ba,0x1b9,0x1b9,0x1b7,0x1b7,0x1b6,0x1b6,
0x1b5,0x1b5,0x1b3,0x1b3,0x1b2,0x1b2,0x1b0,0x1b0,0x1af,0x1af,0x1ae,0x1ae,0x1ac,0x1ac,0x1ab,0x1ab,
0x1aa,0x1aa,0x1a8,0x1a8,0x1a7,0x1a7,0x1a6,0x1a6,0x1a4,0x1a4,0x1a3,0x1a3,0x1a2,0x1a2,0x1a0,0x1a0,
0x19f,0x19f,0x19e,0x19e,0x19c,0x19c,0x19b,0x19b,0x19a,0x19a,0x198,0x198,0x197,0x197,0x196,0x196,
0x194,0x194,0x193,0x193,0x192,0x192,0x190,0x190,0x18f,0x18f,0x18e,0x18e,0x18c,0x18c,0x18b,0x18b,
0x18a,0x18a,0x188,0x188,0x187,0x187,0x186,0x186,0x185,0x185,0x183,0x183,0x182,0x182,0x181,0x181,
0x17f,0x17f,0x17e,0x17e,0x17d,0x17d,0x17b,0x17b,0x17a,0x17a,0x179,0x179,0x177,0x177,0x176,0x176,
0x175,0x175,0x174,0x174,0x172,0x172,0x171,0x171,0x170,0x170,0x16e,0x16e,0x16d,0x16d,0x16c,0x16c,
0x16b,0x16b,0x169,0x169,0x168,0x168,0x167,0x167,0x165,0x165,0x164,0x164,0x163,0x163,0x162,0x162,
0x160,0x160,0x15f,0x15f,0x15e,0x15e,0x15c,0x15c,0x15b,0x15b,0x15a,0x15a,0x159,0x159,0x157,0x157,
0x156,0x156,0x155,0x155,0x153,0x153,0x152,0x152,0x151,0x151,0x150,0x150,0x14e,0x14e,0x14d,0x14d,
0x14c,0x14c,0x14b,0x14b,0x149,0x149,0x148,0x148,0x147,0x147,0x146,0x146,0x144,0x144,0x143,0x143,
0x142,0x142,0x141,0x141,0x13f,0x13f,0x13e,0x13e,0x13d,0x13d,0x13c,0x13c,0x13a,0x13a,0x139,0x139,
0x138,0x138,0x137,0x137,0x135,0x135,0x134,0x134,0x133,0x133,0x132,0x132,0x130,0x130,0x12f,0x12f,
0x12e,0x12e,0x12d,0x12d,0x12b,0x12b,0x12a,0x12a,0x129,0x129,0x128,0x128,0x126,0x126,0x125,0x125,
0x124,0x124,0x123,0x123,0x122,0x122,0x120,0x120,0x11f,0x11f,0x11e,0x11e,0x11d,0x11d,0x11b,0x11b,
0x11a,0x11a,0x119,0x119,0x118,0x118,0x117,0x117,0x115,0x115,0x114,0x114,0x113,0x113,0x112,0x112,
0x110,0x110,0x10f,0x10f,0x10e,0x10e,0x10d,0x10d,0x10c,0x10c,0x10a,0x10a,0x109,0x109,0x108,0x108,
0x107,0x107,0x106,0x106,0x104,0x104,0x103,0x103,0x102,0x102,0x101,0x101,0x100,0x100,0x0fe,0x0fe,
0x0fd,0x0fd,0x0fc,0x0fc,0x0fb,0x0fb,0x0fa,0x0fa,0x0f8,0x0f8,0x0f7,0x0f7,0x0f6,0x0f6,0x0f5,0x0f5,
0x0f4,0x0f4,0x0f2,0x0f2,0x0f1,0x0f1,0x0f0,0x0f0,0x0ef,0x0ef,0x0ee,0x0ee,0x0ed,0x0ed,0x0eb,0x0eb,
0x0ea,0x0ea,0x0e9,0x0e9,0x0e8,0x0e8,0x0e7,0x0e7,0x0e5,0x0e5,0x0e4,0x0e4,0x0e3,0x0e3,0x0e2,0x0e2,
0x0e1,0x0e1,0x0e0,0x0e0,0x0de,0x0de,0x0dd,0x0dd,0x0dc,0x0dc,0x0db,0x0db,0x0da,0x0da,0x0d9,0x0d9,
0x0d7,0x0d7,0x0d6,0x0d6,0x0d5,0x0d5,0x0d4,0x0d4,0x0d3,0x0d3,0x0d2,0x0d2,0x0d0,0x0d0,0x0cf,0x0cf,
0x0ce,0x0ce,0x0cd,0x0cd,0x0cc,0x0cc,0x0cb,0x0cb,0x0c9,0x0c9,0x0c8,0x0c8,0x0c7,0x0c7,0x0c6,0x0c6,
0x0c5,0x0c5,0x0c4,0x0c4,0x0c3,0x0c3,0x0c1,0x0c1,0x0c0,0x0c0,0x0bf,0x0bf,0x0be,0x0be,0x0bd,0x0bd,
0x0bc,0x0bc,0x0bb,0x0bb,0x0b9,0x0b9,0x0b8,0x0b8,0x0b7,0x0b7,0x0b6,0x0b6,0x0b5,0x0b5,0x0b4,0x0b4,
0x0b3,0x0b3,0x0b1,0x0b1,0x0b0,0x0b0,0x0af,0x0af,0x0ae,0x0ae,0x0ad,0x0ad,0x0ac,0x0ac,0x0ab,0x0ab,
0x0a9,0x0a9,0x0a8,0x0a8,0x0a7,0x0a7,0x0a6,0x0a6,0x0a5,0x0a5,0x0a4,0x0a4,0x0a3,0x0a3,0x0a2,0x0a2,
0x0a0,0x0a0,0x09f,0x09f,0x09e,0x09e,0x09d,0x09d,0x09c,0x09c,0x09b,0x09b,0x09a,0x09a,0x099,0x099,
0x098,0x098,0x096,0x096,0x095,0x095,0x094,0x094,0x093,0x093,0x092,0x092,0x091,0x091,0x090,0x090,
0x08f,0x08f,0x08e,0x08e,0x08c,0x08c,0x08b,0x08b,0x08a,0x08a,0x089,0x089,0x088,0x088,0x087,0x087,
0x086,0x086,0x085,0x085,0x084,0x084,0x082,0x082,0x081,0x081,0x080,0x080,0x07f,0x07f,0x07e,0x07e,
0x07d,0x07d,0x07c,0x07c,0x07b,0x07b,0x07a,0x07a,0x079,0x079,0x078,0x078,0x076,0x076,0x075,0x075,
0x074,0x074,0x073,0x073,0x072,0x072,0x071,0x071,0x070,0x070,0x06f,0x06f,0x06e,0x06e,0x06d,0x06d,
0x06c,0x06c,0x06b,0x06b,0x069,0x069,0x068,0x068,0x067,0x067,0x066,0x066,0x065,0x065,0x064,0x064,
0x063,0x063,0x062,0x062,0x061,0x061,0x060,0x060,0x05f,0x05f,0x05e,0x05e,0x05d,0x05d,0x05b,0x05b,
0x05a,0x05a,0x059,0x059,0x058,0x058,0x057,0x057,0x056,0x056,0x055,0x055,0x054,0x054,0x053,0x053,
0x052,0x052,0x051,0x051,0x050,0x050,0x04f,0x04f,0x04e,0x04e,0x04d,0x04d,0x04c,0x04c,0x04a,0x04a,
0x049,0x049,0x048,0x048,0x047,0x047,0x046,0x046,0x045,0x045,0x044,0x044,0x043,0x043,0x042,0x042,
0x041,0x041,0x040,0x040,0x03f,0x03f,0x03e,0x03e,0x03d,0x03d,0x03c,0x03c,0x03b,0x03b,0x03a,0x03a,
0x039,0x039,0x038,0x038,0x037,0x037,0x036,0x036,0x034,0x034,0x033,0x033,0x032,0x032,0x031,0x031,
0x030,0x030,0x02f,0x02f,0x02e,0x02e,0x02d,0x02d,0x02c,0x02c,0x02b,0x02b,0x02a,0x02a,0x029,0x029,
0x028,0x028,0x027,0x027,0x026,0x026,0x025,0x025,0x024,0x024,0x023,0x023,0x022,0x022,0x021,0x021,
0x020,0x020,0x01f,0x01f,0x01e,0x01e,0x01d,0x01d,0x01c,0x01c,0x01b,0x01b,0x01a,0x01a,0x019,0x019,
0x018,0x018,0x017,0x017,0x016,0x016,0x015,0x015,0x014,0x014,0x013,0x013,0x012,0x012,0x011,0x011,
0x010,0x010,0x00f,0x00f,0x00e,0x00e,0x00d,0x00d,0x00c,0x00c,0x00b,0x00b,0x00a,0x00a,0x009,0x009,
0x008,0x008,0x007,0x007,0x006,0x006,0x005,0x005,0x004,0x004,0x003,0x003,0x002,0x002,0x001,0x001,
0xffe,0xffe,0xffa,0xffa,0xff6,0xff6,0xff2,0xff2,0xfee,0xfee,0xfea,0xfea,0xfe6,0xfe6,0xfe2,0xfe2,
0xfde,0xfde,0xfda,0xfda,0xfd6,0xfd6,0xfd2,0xfd2,0xfce,0xfce,0xfcb,0xfcb,0xfc7,0xfc7,0xfc3,0xfc3,
0xfbf,0xfbf,0xfbb,0xfbb,0xfb7,0xfb7,0xfb3,0xfb3,0xfaf,0xfaf,0xfab,0xfab,0xfa7,0xfa7,0xfa4,0xfa4,
0xfa0,0xfa0,0xf9c,0xf9c,0xf98,0xf98,0xf94,0xf94,0xf90,0xf90,0xf8c,0xf8c,0xf89,0xf89,0xf85,0xf85,
0xf81,0xf81,0xf7d,0xf7d,0xf79,0xf79,0xf76,0xf76,0xf72,0xf72,0xf6e,0xf6e,0xf6a,0xf6a,0xf66,0xf66,
0xf63,0xf63,0xf5f,0xf5f,0xf5b,0xf5b,0xf57,0xf57,0xf54,0xf54,0xf50,0xf50,0xf4c,0xf4c,0xf48,0xf48,
0xf45,0xf45,0xf41,0xf41,0xf3d,0xf3d,0xf39,0xf39,0xf36,0xf36,0xf32,0xf32,0xf2e,0xf2e,0xf2b,0xf2b,
0xf27,0xf27,0xf23,0xf23,0xf20,0xf20,0xf1c,0xf1c,0xf18,0xf18,0xf15,0xf15,0xf11,0xf11,0xf0d,0xf0d,
0xf0a,0xf0a,0xf06,0xf06,0xf02,0xf02,0xeff,0xeff,0xefb,0xefb,0xef7,0xef7,0xef4,0xef4,0xef0,0xef0,
0xeed,0xeed,0xee9,0xee9,0xee5,0xee5,0xee2,0xee2,0xede,0xede,0xedb,0xedb,0xed7,0xed7,0xed3,0xed3,
0xed0,0xed0,0xecc,0xecc,0xec9,0xec9,0xec5,0xec5,0xec2,0xec2,0xebe,0xebe,0xeba,0xeba,0xeb7,0xeb7,
0xeb3,0xeb3,0xeb0,0xeb0,0xeac,0xeac,0xea9,0xea9,0xea5,0xea5,0xea2,0xea2,0xe9e,0xe9e,0xe9b,0xe9b,
0xe97,0xe97,0xe94,0xe94,0xe90,0xe90,0xe8d,0xe8d,0xe89,0xe89,0xe86,0xe86,0xe82,0xe82,0xe7f,0xe7f,
0xe7b,0xe7b,0xe78,0xe78,0xe75,0xe75,0xe71,0xe71,0xe6e,0xe6e,0xe6a,0xe6a,0xe67,0xe67,0xe63,0xe63,
0xe60,0xe60,0xe5d,0xe5d,0xe59,0xe59,0xe56,0xe56,0xe52,0xe52,0xe4f,0xe4f,0xe4c,0xe4c,0xe48,0xe48,
0xe45,0xe45,0xe41,0xe41,0xe3e,0xe3e,0xe3b,0xe3b,0xe37,0xe37,0xe34,0xe34,0xe31,0xe31,0xe2d,0xe2d,
0xe2a,0xe2a,0xe26,0xe26,0xe23,0xe23,0xe20,0xe20,0xe1c,0xe1c,0xe19,0xe19,0xe16,0xe16,0xe12,0xe12,
0xe0f,0xe0f,0xe0c,0xe0c,0xe09,0xe09,0xe05,0xe05,0xe02,0xe02,0xdff,0xdff,0xdfb,0xdfb,0xdf8,0xdf8,
0xdf5,0xdf5,0xdf1,0xdf1,0xdee,0xdee,0xdeb,0xdeb,0xde8,0xde8,0xde4,0xde4,0xde1,0xde1,0xdde,0xdde,
0xddb,0xddb,0xdd7,0xdd7,0xdd4,0xdd4,0xdd1,0xdd1,0xdce,0xdce,0xdca,0xdca,0xdc7,0xdc7,0xdc4,0xdc4,
0xdc1,0xdc1,0xdbe,0xdbe,0xdba,0xdba,0xdb7,0xdb7,0xdb4,0xdb4,0xdb1,0xdb1,0xdae,0xdae,0xdaa,0xdaa,
0xda7,0xda7,0xda4,0xda4,0xda1,0xda1,0xd9e,0xd9e,0xd9b,0xd9b,0xd97,0xd97,0xd94,0xd94,0xd91,0xd91,
0xd8e,0xd8e,0xd8b,0xd8b,0xd88,0xd88,0xd84,0xd84,0xd81,0xd81,0xd7e,0xd7e,0xd7b,0xd7b,0xd78,0xd78,
0xd75,0xd75,0xd72,0xd72,0xd6f,0xd6f,0xd6b,0xd6b,0xd68,0xd68,0xd65,0xd65,0xd62,0xd62,0xd5f,0xd5f,
0xd5c,0xd5c,0xd59,0xd59,0xd56,0xd56,0xd53,0xd53,0xd50,0xd50,0xd4d,0xd4d,0xd49,0xd49,0xd46,0xd46,
0xd43,0xd43,0xd40,0xd40,0xd3d,0xd3d,0xd3a,0xd3a,0xd37,0xd37,0xd34,0xd34,0xd31,0xd31,0xd2e,0xd2e,
0xd2b,0xd2b,0xd28,0xd28,0xd25,0xd25,0xd22,0xd22,0xd1f,0xd1f,0xd1c,0xd1c,0xd19,0xd19,0xd16,0xd16,
0xd13,0xd13,0xd10,0xd10,0xd0d,0xd0d,0xd0a,0xd0a,0xd07,0xd07,0xd04,0xd04,0xd01,0xd01,0xcfe,0xcfe,
0xcfb,0xcfb,0xcf8,0xcf8,0xcf5,0xcf5,0xcf2,0xcf2,0xcef,0xcef,0xcec,0xcec,0xce9,0xce9,0xce6,0xce6,
0xce3,0xce3,0xce0,0xce0,0xcdd,0xcdd,0xcdb,0xcdb,0xcd8,0xcd8,0xcd5,0xcd5,0xcd2,0xcd2,0xccf,0xccf,
0xccc,0xccc,0xcc9,0xcc9,0xcc6,0xcc6,0xcc3,0xcc3,0xcc0,0xcc0,0xcbd,0xcbd,0xcba,0xcba,0xcb8,0xcb8,
0xcb5,0xcb5,0xcb2,0xcb2,0xcaf,0xcaf,0xcac,0xcac,0xca9,0xca9,0xca6,0xca6,0xca3,0xca3,0xca1,0xca1,
0xc9e,0xc9e,0xc9b,0xc9b,0xc98,0xc98,0xc95,0xc95,0xc92,0xc92,0xc8f,0xc8f,0xc8d,0xc8d,0xc8a,0xc8a,
0xc87,0xc87,0xc84,0xc84,0xc81,0xc81,0xc7e,0xc7e,0xc7c,0xc7c,0xc79,0xc79,0xc76,0xc76,0xc73,0xc73,
0xc70,0xc70,0xc6e,0xc6e,0xc6b,0xc6b,0xc68,0xc68,0xc65,0xc65,0xc62,0xc62,0xc60,0xc60,0xc5d,0xc5d,
0xc5a,0xc5a,0xc57,0xc57,0xc54,0xc54,0xc52,0xc52,0xc4f,0xc4f,0xc4c,0xc4c,0xc49,0xc49,0xc47,0xc47,
0xc44,0xc44,0xc41,0xc41,0xc3e,0xc3e,0xc3c,0xc3c,0xc39,0xc39,0xc36,0xc36,0xc33,0xc33,0xc31,0xc31,
0xc2e,0xc2e,0xc2b,0xc2b,0xc28,0xc28,0xc26,0xc26,0xc23,0xc23,0xc20,0xc20,0xc1e,0xc1e,0xc1b,0xc1b,
0xc18,0xc18,0xc15,0xc15,0xc13,0xc13,0xc10,0xc10,0xc0d,0xc0d,0xc0b,0xc0b,0xc08,0xc08,0xc05,0xc05,
0xc03,0xc03,0xc00,0xc00,0xbfd,0xbfd,0xbfb,0xbfb,0xbf8,0xbf8,0xbf5,0xbf5,0xbf3,0xbf3,0xbf0,0xbf0,
0xbed,0xbed,0xbeb,0xbeb,0xbe8,0xbe8,0xbe5,0xbe5,0xbe3,0xbe3,0xbe0,0xbe0,0xbdd,0xbdd,0xbdb,0xbdb,
0xbd8,0xbd8,0xbd5,0xbd5,0xbd3,0xbd3,0xbd0,0xbd0,0xbce,0xbce,0xbcb,0xbcb,0xbc8,0xbc8,0xbc6,0xbc6,
0xbc3,0xbc3,0xbc0,0xbc0,0xbbe,0xbbe,0xbbb,0xbbb,0xbb9,0xbb9,0xbb6,0xbb6,0xbb3,0xbb3,0xbb1,0xbb1,
0xbae,0xbae,0xbac,0xbac,0xba9,0xba9,0xba7,0xba7,0xba4,0xba4,0xba1,0xba1,0xb9f,0xb9f,0xb9c,0xb9c,
0xb9a,0xb9a,0xb97,0xb97,0xb95,0xb95,0xb92,0xb92,0xb8f,0xb8f,0xb8d,0xb8d,0xb8a,0xb8a,0xb88,0xb88,
0xb85,0xb85,0xb83,0xb83,0xb80,0xb80,0xb7e,0xb7e,0xb7b,0xb7b,0xb79,0xb79,0xb76,0xb76,0xb73,0xb73,
0xb71,0xb71,0xb6e,0xb6e,0xb6c,0xb6c,0xb69,0xb69,0xb67,0xb67,0xb64,0xb64,0xb62,0xb62,0xb5f,0xb5f,
0xb5d,0xb5d,0xb5a,0xb5a,0xb58,0xb58,0xb55,0xb55,0xb53,0xb53,0xb50,0xb50,0xb4e,0xb4e,0xb4b,0xb4b,
0xb49,0xb49,0xb46,0xb46,0xb44,0xb44,0xb41,0xb41,0xb3f,0xb3f,0xb3d,0xb3d,0xb3a,0xb3a,0xb38,0xb38,
0xb35,0xb35,0xb33,0xb33,0xb30,0xb30,0xb2e,0xb2e,0xb2b,0xb2b,0xb29,0xb29,0xb26,0xb26,0xb24,0xb24,
0xb22,0xb22,0xb1f,0xb1f,0xb1d,0xb1d,0xb1a,0xb1a,0xb18,0xb18,0xb15,0xb15,0xb13,0xb13,0xb11,0xb11,
0xb0e,0xb0e,0xb0c,0xb0c,0xb09,0xb09,0xb07,0xb07,0xb05,0xb05,0xb02,0xb02,0xb00,0xb00,0xafd,0xafd,
0xafb,0xafb,0xaf9,0xaf9,0xaf6,0xaf6,0xaf4,0xaf4,0xaf1,0xaf1,0xaef,0xaef,0xaed,0xaed,0xaea,0xaea,
0xae8,0xae8,0xae5,0xae5,0xae3,0xae3,0xae1,0xae1,0xade,0xade,0xadc,0xadc,0xada,0xada,0xad7,0xad7,
0xad5,0xad5,0xad3,0xad3,0xad0,0xad0,0xace,0xace,0xacb,0xacb,0xac9,0xac9,0xac7,0xac7,0xac4,0xac4,
0xac2,0xac2,0xac0,0xac0,0xabd,0xabd,0xabb,0xabb,0xab9,0xab9,0xab6,0xab6,0xab4,0xab4,0xab2,0xab2,
0xaaf,0xaaf,0xaad,0xaad,0xaab,0xaab,0xaa8,0xaa8,0xaa6,0xaa6,0xaa4,0xaa4,0xaa2,0xaa2,0xa9f,0xa9f,
0xa9d,0xa9d,0xa9b,0xa9b,0xa98,0xa98,0xa96,0xa96,0xa94,0xa94,0xa91,0xa91,0xa8f,0xa8f,0xa8d,0xa8d,
0xa8b,0xa8b,0xa88,0xa88,0xa86,0xa86,0xa84,0xa84,0xa82,0xa82,0xa7f,0xa7f,0xa7d,0xa7d,0xa7b,0xa7b,
0xa78,0xa78,0xa76,0xa76,0xa74,0xa74,0xa72,0xa72,0xa6f,0xa6f,0xa6d,0xa6d,0xa6b,0xa6b,0xa69,0xa69,
0xa66,0xa66,0xa64,0xa64,0xa62,0xa62,0xa60,0xa60,0xa5d,0xa5d,0xa5b,0xa5b,0xa59,0xa59,0xa57,0xa57,
0xa55,0xa55,0xa52,0xa52,0xa50,0xa50,0xa4e,0xa4e,0xa4c,0xa4c,0xa49,0xa49,0xa47,0xa47,0xa45,0xa45,
0xa43,0xa43,0xa41,0xa41,0xa3e,0xa3e,0xa3c,0xa3c,0xa3a,0xa3a,0xa38,0xa38,0xa36,0xa36,0xa33,0xa33,
0xa31,0xa31,0xa2f,0xa2f,0xa2d,0xa2d,0xa2b,0xa2b,0xa28,0xa28,0xa26,0xa26,0xa24,0xa24,0xa22,0xa22,
0xa20,0xa20,0xa1d,0xa1d,0xa1b,0xa1b,0xa19,0xa19,0xa17,0xa17,0xa15,0xa15,0xa13,0xa13,0xa10,0xa10,
0xa0e,0xa0e,0xa0c,0xa0c,0xa0a,0xa0a,0xa08,0xa08,0xa06,0xa06,0xa04,0xa04,0xa01,0xa01,0x9ff,0x9ff,
0x9fd,0x9fd,0x9fb,0x9fb,0x9f9,0x9f9,0x9f7,0x9f7,0x9f5,0x9f5,0x9f2,0x9f2,0x9f0,0x9f0,0x9ee,0x9ee,
0x9ec,0x9ec,0x9ea,0x9ea,0x9e8,0x9e8,0x9e6,0x9e6,0x9e4,0x9e4,0x9e1,0x9e1,0x9df,0x9df,0x9dd,0x9dd,
0x9db,0x9db,0x9d9,0x9d9,0x9d7,0x9d7,0x9d5,0x9d5,0x9d3,0x9d3,0x9d1,0x9d1,0x9ce,0x9ce,0x9cc,0x9cc,
0x9ca,0x9ca,0x9c8,0x9c8,0x9c6,0x9c6,0x9c4,0x9c4,0x9c2,0x9c2,0x9c0,0x9c0,0x9be,0x9be,0x9bc,0x9bc,
0x9ba,0x9ba,0x9b7,0x9b7,0x9b5,0x9b5,0x9b3,0x9b3,0x9b1,0x9b1,0x9af,0x9af,0x9ad,0x9ad,0x9ab,0x9ab,
0x9a9,0x9a9,0x9a7,0x9a7,0x9a5,0x9a5,0x9a3,0x9a3,0x9a1,0x9a1,0x99f,0x99f,0x99d,0x99d,0x99b,0x99b,
0x999,0x999,0x997,0x997,0x994,0x994,0x992,0x992,0x990,0x990,0x98e,0x98e,0x98c,0x98c,0x98a,0x98a,
0x988,0x988,0x986,0x986,0x984,0x984,0x982,0x982,0x980,0x980,0x97e,0x97e,0x97c,0x97c,0x97a,0x97a,
0x978,0x978,0x976,0x976,0x974,0x974,0x972,0x972,0x970,0x970,0x96e,0x96e,0x96c,0x96c,0x96a,0x96a,
0x968,0x968,0x966,0x966,0x964,0x964,0x962,0x962,0x960,0x960,0x95e,0x95e,0x95c,0x95c,0x95a,0x95a,
0x958,0x958,0x956,0x956,0x954,0x954,0x952,0x952,0x950,0x950,0x94e,0x94e,0x94c,0x94c,0x94a,0x94a,
0x948,0x948,0x946,0x946,0x944,0x944,0x942,0x942,0x940,0x940,0x93e,0x93e,0x93c,0x93c,0x93a,0x93a,
0x938,0x938,0x937,0x937,0x935,0x935,0x933,0x933,0x931,0x931,0x92f,0x92f,0x92d,0x92d,0x92b,0x92b,
0x929,0x929,0x927,0x927,0x925,0x925,0x923,0x923,0x921,0x921,0x91f,0x91f,0x91d,0x91d,0x91b,0x91b,
0x919,0x919,0x917,0x917,0x916,0x916,0x914,0x914,0x912,0x912,0x910,0x910,0x90e,0x90e,0x90c,0x90c,
0x90a,0x90a,0x908,0x908,0x906,0x906,0x904,0x904,0x902,0x902,0x900,0x900,0x8ff,0x8ff,0x8fd,0x8fd,
0x8fb,0x8fb,0x8f9,0x8f9,0x8f7,0x8f7,0x8f5,0x8f5,0x8f3,0x8f3,0x8f1,0x8f1,0x8ef,0x8ef,0x8ed,0x8ed,
0x8ec,0x8ec,0x8ea,0x8ea,0x8e8,0x8e8,0x8e6,0x8e6,0x8e4,0x8e4,0x8e2,0x8e2,0x8e0,0x8e0,0x8de,0x8de,
0x8dc,0x8dc,0x8db,0x8db,0x8d9,0x8d9,0x8d7,0x8d7,0x8d5,0x8d5,0x8d3,0x8d3,0x8d1,0x8d1,0x8cf,0x8cf,
0x8ce,0x8ce,0x8cc,0x8cc,0x8ca,0x8ca,0x8c8,0x8c8,0x8c6,0x8c6,0x8c4,0x8c4,0x8c2,0x8c2,0x8c1,0x8c1,
0x8bf,0x8bf,0x8bd,0x8bd,0x8bb,0x8bb,0x8b9,0x8b9,0x8b7,0x8b7,0x8b5,0x8b5,0x8b4,0x8b4,0x8b2,0x8b2,
0x8b0,0x8b0,0x8ae,0x8ae,0x8ac,0x8ac,0x8aa,0x8aa,0x8a9,0x8a9,0x8a7,0x8a7,0x8a5,0x8a5,0x8a3,0x8a3,
0x8a1,0x8a1,0x89f,0x89f,0x89e,0x89e,0x89c,0x89c,0x89a,0x89a,0x898,0x898,0x896,0x896,0x895,0x895,
0x893,0x893,0x891,0x891,0x88f,0x88f,0x88d,0x88d,0x88c,0x88c,0x88a,0x88a,0x888,0x888,0x886,0x886,
0x884,0x884,0x883,0x883,0x881,0x881,0x87f,0x87f,0x87d,0x87d,0x87b,0x87b,0x87a,0x87a,0x878,0x878,
0x876,0x876,0x874,0x874,0x872,0x872,0x871,0x871,0x86f,0x86f,0x86d,0x86d,0x86b,0x86b,0x86a,0x86a,
0x868,0x868,0x866,0x866,0x864,0x864,0x862,0x862,0x861,0x861,0x85f,0x85f,0x85d,0x85d,0x85b,0x85b,
0x85a,0x85a,0x858,0x858,0x856,0x856,0x854,0x854,0x853,0x853,0x851,0x851,0x84f,0x84f,0x84d,0x84d,
0x84c,0x84c,0x84a,0x84a,0x848,0x848,0x846,0x846,0x845,0x845,0x843,0x843,0x841,0x841,0x83f,0x83f,
0x83e,0x83e,0x83c,0x83c,0x83a,0x83a,0x838,0x838,0x837,0x837,0x835,0x835,0x833,0x833,0x831,0x831,
0x830,0x830,0x82e,0x82e,0x82c,0x82c,0x82b,0x82b,0x829,0x829,0x827,0x827,0x825,0x825,0x824,0x824,
0x822,0x822,0x820,0x820,0x81f,0x81f,0x81d,0x81d,0x81b,0x81b,0x819,0x819,0x818,0x818,0x816,0x816,
0x814,0x814,0x813,0x813,0x811,0x811,0x80f,0x80f,0x80d,0x80d,0x80c,0x80c,0x80a,0x80a,0x808,0x808,
0x807,0x807,0x805,0x805,0x803,0x803,0x802,0x802,0x800,0x800,0x7fe,0x7fe,0x7fd,0x7fd,0x7fb,0x7fb,
0x7f9,0x7f9,0x7f7,0x7f7,0x7f6,0x7f6,0x7f4,0x7f4,0x7f2,0x7f2,0x7f1,0x7f1,0x7ef,0x7ef,0x7ed,0x7ed,
0x7ec,0x7ec,0x7ea,0x7ea,0x7e8,0x7e8,0x7e7,0x7e7,0x7e5,0x7e5,0x7e3,0x7e3,0x7e2,0x7e2,0x7e0,0x7e0,
0x7de,0x7de,0x7dd,0x7dd,0x7db,0x7db,0x7d9,0x7d9,0x7d8,0x7d8,0x7d6,0x7d6,0x7d4,0x7d4,0x7d3,0x7d3,
0x7d1,0x7d1,0x7d0,0x7d0,0x7ce,0x7ce,0x7cc,0x7cc,0x7cb,0x7cb,0x7c9,0x7c9,0x7c7,0x7c7,0x7c6,0x7c6,
0x7c4,0x7c4,0x7c2,0x7c2,0x7c1,0x7c1,0x7bf,0x7bf,0x7be,0x7be,0x7bc,0x7bc,0x7ba,0x7ba,0x7b9,0x7b9,
0x7b7,0x7b7,0x7b5,0x7b5,0x7b4,0x7b4,0x7b2,0x7b2,0x7b0,0x7b0,0x7af,0x7af,0x7ad,0x7ad,0x7ac,0x7ac,
0x7aa,0x7aa,0x7a8,0x7a8,0x7a7,0x7a7,0x7a5,0x7a5,0x7a4,0x7a4,0x7a2,0x7a2,0x7a0,0x7a0,0x79f,0x79f,
0x79d,0x79d,0x79b,0x79b,0x79a,0x79a,0x798,0x798,0x797,0x797,0x795,0x795,0x793,0x793,0x792,0x792,
0x790,0x790,0x78f,0x78f,0x78d,0x78d,0x78b,0x78b,0x78a,0x78a,0x788,0x788,0x787,0x787,0x785,0x785,
0x784,0x784,0x782,0x782,0x780,0x780,0x77f,0x77f,0x77d,0x77d,0x77c,0x77c,0x77a,0x77a,0x778,0x778,
0x777,0x777,0x775,0x775,0x774,0x774,0x772,0x772,0x771,0x771,0x76f,0x76f,0x76d,0x76d,0x76c,0x76c,
0x76a,0x76a,0x769,0x769,0x767,0x767,0x766,0x766,0x764,0x764,0x762,0x762,0x761,0x761,0x75f,0x75f,
0x75e,0x75e,0x75c,0x75c,0x75b,0x75b,0x759,0x759,0x758,0x758,0x756,0x756,0x754,0x754,0x753,0x753,
0x751,0x751,0x750,0x750,0x74e,0x74e,0x74d,0x74d,0x74b,0x74b,0x74a,0x74a,0x748,0x748,0x747,0x747,
0x745,0x745,0x744,0x744,0x742,0x742,0x740,0x740,0x73f,0x73f,0x73d,0x73d,0x73c,0x73c,0x73a,0x73a,
0x739,0x739,0x737,0x737,0x736,0x736,0x734,0x734,0x733,0x733,0x731,0x731,0x730,0x730,0x72e,0x72e,
0x72d,0x72d,0x72b,0x72b,0x72a,0x72a,0x728,0x728,0x727,0x727,0x725,0x725,0x723,0x723,0x722,0x722,
0x720,0x720,0x71f,0x71f,0x71d,0x71d,0x71c,0x71c,0x71a,0x71a,0x719,0x719,0x717,0x717,0x716,0x716,
0x714,0x714,0x713,0x713,0x711,0x711,0x710,0x710,0x70e,0x70e,0x70d,0x70d,0x70b,0x70b,0x70a,0x70a,
0x708,0x708,0x707,0x707,0x705,0x705,0x704,0x704,0x703,0x703,0x701,0x701,0x700,0x700,0x6fe,0x6fe,
0x6fd,0x6fd,0x6fb,0x6fb,0x6fa,0x6fa,0x6f8,0x6f8,0x6f7,0x6f7,0x6f5,0x6f5,0x6f4,0x6f4,0x6f2,0x6f2,
0x6f1,0x6f1,0x6ef,0x6ef,0x6ee,0x6ee,0x6ec,0x6ec,0x6eb,0x6eb,0x6e9,0x6e9,0x6e8,0x6e8,0x6e6,0x6e6,
0x6e5,0x6e5,0x6e4,0x6e4,0x6e2,0x6e2,0x6e1,0x6e1,0x6df,0x6df,0x6de,0x6de,0x6dc,0x6dc,0x6db,0x6db,
0x6d9,0x6d9,0x6d8,0x6d8,0x6d6,0x6d6,0x6d5,0x6d5,0x6d3,0x6d3,0x6d2,0x6d2,0x6d1,0x6d1,0x6cf,0x6cf,
0x6ce,0x6ce,0x6cc,0x6cc,0x6cb,0x6cb,0x6c9,0x6c9,0x6c8,0x6c8,0x6c6,0x6c6,0x6c5,0x6c5,0x6c4,0x6c4,
0x6c2,0x6c2,0x6c1,0x6c1,0x6bf,0x6bf,0x6be,0x6be,0x6bc,0x6bc,0x6bb,0x6bb,0x6ba,0x6ba,0x6b8,0x6b8,
0x6b7,0x6b7,0x6b5,0x6b5,0x6b4,0x6b4,0x6b2,0x6b2,0x6b1,0x6b1,0x6b0,0x6b0,0x6ae,0x6ae,0x6ad,0x6ad,
0x6ab,0x6ab,0x6aa,0x6aa,0x6a8,0x6a8,0x6a7,0x6a7,0x6a6,0x6a6,0x6a4,0x6a4,0x6a3,0x6a3,0x6a1,0x6a1,
];

/// `_mm_rcp_ps` (see the module documentation for the exact model).
#[inline]
pub fn simd_rcp_ps(x: f32) -> f32 {
    let b = x.to_bits();
    let sign = b & 0x8000_0000;
    let e = (b >> 23) & 0xff;
    let m = b & 0x007f_ffff;
    if e == 0 {
        // zero and subnormals
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if e == 0xff {
        if m == 0 {
            return f32::from_bits(sign); // +/-inf -> +/-0
        }
        return f32::from_bits(b | 0x0040_0000); // quiet the NaN
    }
    let re = 253i32 - e as i32;
    if re <= 0 {
        return f32::from_bits(sign); // underflow -> +/-0
    }
    f32::from_bits(sign | ((re as u32) << 23) | ((RCP_TAB[(m >> 12) as usize] as u32) << 11))
}

/// `_mm_rsqrt_ps` (see the module documentation for the exact model).
#[inline]
pub fn simd_rsqrt_ps(x: f32) -> f32 {
    let b = x.to_bits();
    let sign = b & 0x8000_0000;
    let e = (b >> 23) & 0xff;
    let m = b & 0x007f_ffff;
    if e == 0 {
        // zero and subnormals -> +/-inf
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if e == 0xff {
        if m == 0 {
            return if sign != 0 {
                f32::from_bits(0xffc0_0000) // -inf -> SSE indefinite
            } else {
                f32::from_bits(0) // +inf -> +0
            };
        }
        return f32::from_bits(b | 0x0040_0000); // quiet the NaN
    }
    if sign != 0 {
        return f32::from_bits(0xffc0_0000); // negative finite -> SSE indefinite
    }
    let idx = (((e & 1) << 11) | (m >> 12)) as usize;
    let re = 190i32 - ((e as i32 + 1) >> 1);
    f32::from_bits(((re as u32) << 23) | ((RSQRT_TAB[idx] as u32) << 11))
}

/// [`simd_rcp_ps`] over four lanes.
///
/// On x86_64 the hardware `_mm_rcp_ps` is used directly.  [`simd_rcp_ps`] is an
/// exact software model of that instruction (bit-for-bit over 8,000,000 tested
/// inputs, including denormals, ±Inf, NaN and negatives), but the model costs
/// ~15 scalar operations per lane where the C executes one instruction for all
/// four; the PVQ search calls this once per band, so the difference is small
/// here but the swap is free.  The portable model remains the fallback.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn simd_rcp_ps_v4(a: [f32; 4]) -> [f32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        // SAFETY: SSE is part of the x86_64 baseline.  The load/store touch
        // exactly the four elements of `a` / `o`.
        let v = unsafe { _mm_rcp_ps(_mm_loadu_ps(a.as_ptr())) };
        let mut o = [0.0f32; 4];
        unsafe { _mm_storeu_ps(o.as_mut_ptr(), v) };
        o
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        [
            simd_rcp_ps(a[0]),
            simd_rcp_ps(a[1]),
            simd_rcp_ps(a[2]),
            simd_rcp_ps(a[3]),
        ]
    }
}

/// [`simd_rsqrt_ps`] over four lanes.
///
/// On x86_64 the hardware `_mm_rsqrt_ps` is used directly.  [`simd_rsqrt_ps`] is
/// an exact software model of that instruction (bit-for-bit over 8,000,000
/// tested inputs, including denormals, ±Inf, NaN and negatives), but it costs
/// ~15 scalar operations per lane (bit manipulation plus an `RSQRT_TAB`
/// lookup).  `op_pvq_search_sse2` calls this once per 4 samples *per pulse*, so
/// the model ran O(K·N) emulated square-root reciprocals where the C runs one
/// `rsqrtps`; that is the dominant cost of the PVQ search.  The portable model
/// remains the fallback.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
#[inline(always)]
fn simd_rsqrt_ps_v4(a: [f32; 4]) -> [f32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        // SAFETY: SSE is part of the x86_64 baseline.  The load/store touch
        // exactly the four elements of `a` / `o`.
        let v = unsafe { _mm_rsqrt_ps(_mm_loadu_ps(a.as_ptr())) };
        let mut o = [0.0f32; 4];
        unsafe { _mm_storeu_ps(o.as_mut_ptr(), v) };
        o
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        [
            simd_rsqrt_ps(a[0]),
            simd_rsqrt_ps(a[1]),
            simd_rsqrt_ps(a[2]),
            simd_rsqrt_ps(a[3]),
        ]
    }
}

// ---------------------------------------------------------------------------
// xcorr_kernel
// ---------------------------------------------------------------------------

/// `celt/x86/pitch_sse.c:43` `xcorr_kernel_sse`.
///
/// `sum[k] += Σ_{m<len} x[m]·y[m+k]` for `k = 0..3`, using two partial
/// accumulators: `xsum1` takes the `m ≡ j` and `m ≡ j+2` terms of each 4-block
/// (`j` the block start), `xsum2` the `m ≡ j+1` and `m ≡ j+3` terms, and the two
/// are added lane-wise once at the very end.
///
/// # Slice lengths
/// `x.len() >= len`, `y.len() >= len + 3` (when `len > 0`): the main loop's
/// `_mm_loadu_ps(y+j+3)` and the tail's `_mm_loadu_ps(y+j)` read up to
/// `y[len+2]`.  The C callers allocate with 3 extra elements (e.g. `celt_fir`,
/// `op_pvq_search`).
///
/// With `--features fast` and AVX2+FMA present, `len >= 24` is handled by
/// [`xcorr_kernel_avx2_fma_hw`]: four 8-lane FMA accumulators instead of the
/// two 4-lane mul+add accumulators below.
pub fn xcorr_kernel(x: &[f32], y: &[f32], sum: &mut [f32; 4], len: i32) {
    debug_assert!(len >= 0);
    debug_assert!(x.len() >= len as usize, "xcorr_kernel: x needs len elements");
    if len > 0 {
        debug_assert!(
            y.len() >= len as usize + 3,
            "xcorr_kernel: y needs len+3 elements"
        );
    }
    #[cfg(all(feature = "fast", target_arch = "x86_64"))]
    {
        /* `len >= 24`: the four 8-lane horizontal reductions dominate below
           that (measured len=8 7.3 -> 11.0 ns, len=24 16.4 -> 14.6 ns). */
        if len >= 24 && have_avx2_fma() {
            /* SAFETY: `have_avx2_fma()` guarantees AVX2+FMA and the asserts
               above give the slice contract of the variant. */
            unsafe { xcorr_kernel_avx2_fma_hw(x, y, sum, len) };
            return;
        }
    }
    /* SSE is part of the x86-64 baseline (`OPUS_X86_PRESUME_SSE` in the
       reference build), so the C body is transcribed instruction for
       instruction: two accumulators, `_mm_shuffle_ps` lane selection, folded
       once at the end.  The `[f32; 4]` emulation below stays as the fallback. */
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        /* SAFETY: SSE is the x86-64 baseline.  The debug asserts above give
           `x.len() >= len`, `y.len() >= len + 3`; the loop's furthest load is
           `y[j+3 .. j+7]` with `j <= len-4` (so `y[len-1 .. len+3]`) and the
           tail's is `y[j .. j+4]` with `j <= len-1` (so `y[len-1 .. len+3]`).
           The store writes exactly `sum[0..4]`. */
        unsafe {
            let mut xsum1 = _mm_loadu_ps(sum.as_ptr());
            let mut xsum2 = _mm_setzero_ps();
            let mut j: i32 = 0;
            while j < len - 3 {
                let jj = j as usize;
                let x0 = _mm_loadu_ps(x.as_ptr().add(jj));
                let yj = _mm_loadu_ps(y.as_ptr().add(jj));
                let y3 = _mm_loadu_ps(y.as_ptr().add(jj + 3));
                xsum1 = _mm_add_ps(
                    xsum1,
                    _mm_mul_ps(_mm_shuffle_ps(x0, x0, 0x00), yj),
                );
                xsum2 = _mm_add_ps(
                    xsum2,
                    _mm_mul_ps(
                        _mm_shuffle_ps(x0, x0, 0x55),
                        _mm_shuffle_ps(yj, y3, 0x49),
                    ),
                );
                xsum1 = _mm_add_ps(
                    xsum1,
                    _mm_mul_ps(
                        _mm_shuffle_ps(x0, x0, 0xaa),
                        _mm_shuffle_ps(yj, y3, 0x9e),
                    ),
                );
                xsum2 = _mm_add_ps(xsum2, _mm_mul_ps(_mm_shuffle_ps(x0, x0, 0xff), y3));
                j += 4;
            }
            if j < len {
                let mut jj = j as usize;
                xsum1 = _mm_add_ps(
                    xsum1,
                    _mm_mul_ps(
                        _mm_load1_ps(x.as_ptr().add(jj)),
                        _mm_loadu_ps(y.as_ptr().add(jj)),
                    ),
                );
                j += 1;
                if j < len {
                    jj = j as usize;
                    xsum2 = _mm_add_ps(
                        xsum2,
                        _mm_mul_ps(
                            _mm_load1_ps(x.as_ptr().add(jj)),
                            _mm_loadu_ps(y.as_ptr().add(jj)),
                        ),
                    );
                    j += 1;
                    if j < len {
                        jj = j as usize;
                        xsum1 = _mm_add_ps(
                            xsum1,
                            _mm_mul_ps(
                                _mm_load1_ps(x.as_ptr().add(jj)),
                                _mm_loadu_ps(y.as_ptr().add(jj)),
                            ),
                        );
                    }
                }
            }
            _mm_storeu_ps(sum.as_mut_ptr(), _mm_add_ps(xsum1, xsum2));
        }
        return;
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut xsum1 = *sum;
        let mut xsum2 = [0.0f32; 4];
        let mut j: i32 = 0;
        while j < len - 3 {
            let jj = j as usize;
            let x0 = v4_load(x, jj);
            let yj = v4_load(y, jj);
            let y3 = v4_load(y, jj + 3);
            xsum1 = v4_add(xsum1, v4_mul(splat4(x0[0]), yj));
            xsum2 = v4_add(xsum2, v4_mul(splat4(x0[1]), shuffle_ps(yj, y3, 0x49)));
            xsum1 = v4_add(xsum1, v4_mul(splat4(x0[2]), shuffle_ps(yj, y3, 0x9e)));
            xsum2 = v4_add(xsum2, v4_mul(splat4(x0[3]), y3));
            j += 4;
        }
        if j < len {
            let jj = j as usize;
            xsum1 = v4_add(xsum1, v4_mul(splat4(x[jj]), v4_load(y, jj)));
            j += 1;
            if j < len {
                let jj = j as usize;
                xsum2 = v4_add(xsum2, v4_mul(splat4(x[jj]), v4_load(y, jj)));
                j += 1;
                if j < len {
                    let jj = j as usize;
                    xsum1 = v4_add(xsum1, v4_mul(splat4(x[jj]), v4_load(y, jj)));
                }
            }
        }
        *sum = v4_add(xsum1, xsum2);
    }
}

/// `celt/pitch.h:65` `xcorr_kernel_c` — the plain-C reference (not reachable in
/// the reference configuration; used by the probe to document the difference).
///
/// `sum[k] += Σ_{m<len} x[m]·y[m+k]` accumulating strictly in ascending `m`.
/// The C reloads `y_0..y_2` one iteration ahead, so it also reads up to 3
/// elements past `y[len-1]`: `y.len() >= len + 3`.
pub fn xcorr_kernel_c(x: &[f32], y: &[f32], sum: &mut [f32; 4], len: i32) {
    debug_assert!(len >= 3, "xcorr_kernel_c asserts len>=3");
    debug_assert!(x.len() >= len as usize);
    debug_assert!(
        y.len() >= len as usize + 3,
        "xcorr_kernel_c: y needs len+3 elements"
    );
    let mut xp = 0usize;
    let mut yp = 3usize;
    let mut y_0 = y[0];
    let mut y_1 = y[1];
    let mut y_2 = y[2];
    let mut y_3 = 0.0f32;
    let mut j: i32 = 0;
    while j < len - 3 {
        let mut tmp = x[xp];
        xp += 1;
        y_3 = y[yp];
        yp += 1;
        sum[0] = MAC16_16(sum[0], tmp, y_0);
        sum[1] = MAC16_16(sum[1], tmp, y_1);
        sum[2] = MAC16_16(sum[2], tmp, y_2);
        sum[3] = MAC16_16(sum[3], tmp, y_3);
        tmp = x[xp];
        xp += 1;
        y_0 = y[yp];
        yp += 1;
        sum[0] = MAC16_16(sum[0], tmp, y_1);
        sum[1] = MAC16_16(sum[1], tmp, y_2);
        sum[2] = MAC16_16(sum[2], tmp, y_3);
        sum[3] = MAC16_16(sum[3], tmp, y_0);
        tmp = x[xp];
        xp += 1;
        y_1 = y[yp];
        yp += 1;
        sum[0] = MAC16_16(sum[0], tmp, y_2);
        sum[1] = MAC16_16(sum[1], tmp, y_3);
        sum[2] = MAC16_16(sum[2], tmp, y_0);
        sum[3] = MAC16_16(sum[3], tmp, y_1);
        tmp = x[xp];
        xp += 1;
        y_2 = y[yp];
        yp += 1;
        sum[0] = MAC16_16(sum[0], tmp, y_3);
        sum[1] = MAC16_16(sum[1], tmp, y_0);
        sum[2] = MAC16_16(sum[2], tmp, y_1);
        sum[3] = MAC16_16(sum[3], tmp, y_2);
        j += 4;
    }
    // The C test is `if (j++<len)`: `j` is post-incremented even when it fails.
    if j < len {
        let tmp = x[xp];
        xp += 1;
        j += 1;
        y_3 = y[yp];
        yp += 1;
        sum[0] = MAC16_16(sum[0], tmp, y_0);
        sum[1] = MAC16_16(sum[1], tmp, y_1);
        sum[2] = MAC16_16(sum[2], tmp, y_2);
        sum[3] = MAC16_16(sum[3], tmp, y_3);
    } else {
        j += 1;
    }
    if j < len {
        let tmp = x[xp];
        xp += 1;
        j += 1;
        y_0 = y[yp];
        yp += 1;
        sum[0] = MAC16_16(sum[0], tmp, y_1);
        sum[1] = MAC16_16(sum[1], tmp, y_2);
        sum[2] = MAC16_16(sum[2], tmp, y_3);
        sum[3] = MAC16_16(sum[3], tmp, y_0);
    } else {
        j += 1;
    }
    if j < len {
        let tmp = x[xp];
        y_1 = y[yp];
        sum[0] = MAC16_16(sum[0], tmp, y_2);
        sum[1] = MAC16_16(sum[1], tmp, y_3);
        sum[2] = MAC16_16(sum[2], tmp, y_0);
        sum[3] = MAC16_16(sum[3], tmp, y_1);
    }
}

// ---------------------------------------------------------------------------
// celt_inner_prod / dual_inner_prod
// ---------------------------------------------------------------------------

/// `celt/x86/pitch_sse.c:108` `celt_inner_prod_sse`, transcribed with
/// `core::arch::x86_64` intrinsics.
///
/// The portable `[f32; 4]` lane emulation is arithmetically identical but
/// LLVM will not vectorise it (the same problem documented for
/// [`xcorr_kernel_avx_hw`]): every block becomes scalar `movss`/`mulss`/`addss`
/// plus bounds comparisons, where the C runs one `movups`/`mulps`/`addps` per
/// operand.  SSE/SSE2 are part of the x86-64 baseline, so no `#[target_feature]`
/// is required.  Only lane 0 of the final `sum` is observed, exactly as in the
/// C, and the tail keeps `MAC16_16` (mul then add, never fused).
///
/// `x.len() >= n >= 0` and `y.len() >= n` (no over-read).
#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn celt_inner_prod_sse_hw(x: &[f32], y: &[f32], n: i32) -> f32 {
    use core::arch::x86_64::*;

    // SAFETY: the vector loop runs while `i < n - 3` and loads
    // `x[i..i+4]`/`y[i..i+4]`, which the caller guarantees to be in bounds
    // (`x.len()`, `y.len() >= n`); the tail is scalar and `i < n` there.
    unsafe {
        let mut sum = _mm_setzero_ps();
        let mut i: i32 = 0;
        while i < n - 3 {
            let ii = i as usize;
            sum = _mm_add_ps(
                sum,
                _mm_mul_ps(
                    _mm_loadu_ps(x.as_ptr().add(ii)),
                    _mm_loadu_ps(y.as_ptr().add(ii)),
                ),
            );
            i += 4;
        }
        // `_mm_add_ps(sum, _mm_movehl_ps(sum,sum))` then `_mm_add_ss` with
        // `_mm_shuffle_ps(sum,sum,0x55)`: lane 0 becomes (s0+s2) + (s1+s3).
        sum = _mm_add_ps(sum, _mm_movehl_ps(sum, sum));
        sum = _mm_add_ss(sum, _mm_shuffle_ps::<0x55>(sum, sum));
        let mut xy = _mm_cvtss_f32(sum);
        while i < n {
            let ii = i as usize;
            xy = MAC16_16(xy, *x.get_unchecked(ii), *y.get_unchecked(ii));
            i += 1;
        }
        xy
    }
}

// ---------------------------------------------------------------------------
// [fast] AVX2+FMA variants
//
// Everything in this section is compiled only with `--features fast` and is
// reached only behind the run-time `have_avx2_fma()` check, so a CPU without
// FMA keeps executing the bit-exact bodies above.  The variants deliberately
// change the floating-point operation order (8 lanes instead of 4, fused
// multiply-add instead of mul+add) and are therefore *not* bit-identical to
// libopus; see `docs/progress.md`.
// ---------------------------------------------------------------------------

/// `[fast]` 8-lane AVX2+FMA `celt_inner_prod`: four `__m256` accumulator
/// chains (32 samples per iteration) to hide the 5-cycle FMA latency — a single
/// chain is latency-bound and barely faster than the 4-lane SSE kernel — folded
/// as `(s0+s1)+(s2+s3)` and then with the 8→4→1 tree
/// `((s0+s4)+(s2+s6)) + ((s1+s5)+(s3+s7))`, followed by the `N mod 8` tail with
/// scalar `MAC16_16`.  Requires `n >= 8`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
#[target_feature(enable = "avx2,fma")]
unsafe fn celt_inner_prod_avx2_fma_hw(x: &[f32], y: &[f32], n: i32) -> f32 {
    use core::arch::x86_64::*;
    // SAFETY: the caller checks `n >= 8` and `x.len()`, `y.len() >= n`.  Every
    // 8-lane load starts at `i + lane_block*8` with `i + lane_block*8 + 8 <= n`.
    unsafe {
        let mut s = [_mm256_setzero_ps(); 4];
        let mut i: i32 = 0;
        while i < n - 31 {
            let ii = i as usize;
            s[0] = _mm256_fmadd_ps(
                _mm256_loadu_ps(x.as_ptr().add(ii)),
                _mm256_loadu_ps(y.as_ptr().add(ii)),
                s[0],
            );
            s[1] = _mm256_fmadd_ps(
                _mm256_loadu_ps(x.as_ptr().add(ii + 8)),
                _mm256_loadu_ps(y.as_ptr().add(ii + 8)),
                s[1],
            );
            s[2] = _mm256_fmadd_ps(
                _mm256_loadu_ps(x.as_ptr().add(ii + 16)),
                _mm256_loadu_ps(y.as_ptr().add(ii + 16)),
                s[2],
            );
            s[3] = _mm256_fmadd_ps(
                _mm256_loadu_ps(x.as_ptr().add(ii + 24)),
                _mm256_loadu_ps(y.as_ptr().add(ii + 24)),
                s[3],
            );
            i += 32;
        }
        while i < n - 7 {
            let ii = i as usize;
            s[0] = _mm256_fmadd_ps(
                _mm256_loadu_ps(x.as_ptr().add(ii)),
                _mm256_loadu_ps(y.as_ptr().add(ii)),
                s[0],
            );
            i += 8;
        }
        let sum = _mm256_add_ps(_mm256_add_ps(s[0], s[1]), _mm256_add_ps(s[2], s[3]));
        let lo = _mm256_castps256_ps128(sum);
        let hi = _mm256_extractf128_ps::<1>(sum);
        let mut r = _mm_add_ps(lo, hi);
        r = _mm_add_ps(r, _mm_movehl_ps(r, r));
        r = _mm_add_ss(r, _mm_shuffle_ps::<0x55>(r, r));
        let mut xy = _mm_cvtss_f32(r);
        while i < n {
            let ii = i as usize;
            xy = MAC16_16(xy, *x.get_unchecked(ii), *y.get_unchecked(ii));
            i += 1;
        }
        xy
    }
}

/// `[fast]` 8-lane AVX2+FMA `dual_inner_prod`: two accumulators per product
/// (four independent FMA chains), so the loop is throughput- rather than
/// latency-bound; each product is reduced as in
/// [`celt_inner_prod_avx2_fma_hw`], then the `N mod 8` tail is added for both.
/// Requires `n >= 8`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
#[target_feature(enable = "avx2,fma")]
unsafe fn dual_inner_prod_avx2_fma_hw(
    x: &[f32],
    y01: &[f32],
    y02: &[f32],
    n: i32,
    xy1: &mut f32,
    xy2: &mut f32,
) {
    use core::arch::x86_64::*;
    // SAFETY: the caller checks `n >= 8` and that all three slices hold at
    // least `n` elements; every 8-lane load stays inside them.
    unsafe {
        let mut s1 = [_mm256_setzero_ps(); 2];
        let mut s2 = [_mm256_setzero_ps(); 2];
        let mut i: i32 = 0;
        while i < n - 15 {
            let ii = i as usize;
            let x0 = _mm256_loadu_ps(x.as_ptr().add(ii));
            s1[0] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y01.as_ptr().add(ii)), s1[0]);
            s2[0] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y02.as_ptr().add(ii)), s2[0]);
            let x1 = _mm256_loadu_ps(x.as_ptr().add(ii + 8));
            s1[1] = _mm256_fmadd_ps(x1, _mm256_loadu_ps(y01.as_ptr().add(ii + 8)), s1[1]);
            s2[1] = _mm256_fmadd_ps(x1, _mm256_loadu_ps(y02.as_ptr().add(ii + 8)), s2[1]);
            i += 16;
        }
        while i < n - 7 {
            let ii = i as usize;
            let xi = _mm256_loadu_ps(x.as_ptr().add(ii));
            s1[0] = _mm256_fmadd_ps(xi, _mm256_loadu_ps(y01.as_ptr().add(ii)), s1[0]);
            s2[0] = _mm256_fmadd_ps(xi, _mm256_loadu_ps(y02.as_ptr().add(ii)), s2[0]);
            i += 8;
        }
        let v1 = _mm256_add_ps(s1[0], s1[1]);
        let v2 = _mm256_add_ps(s2[0], s2[1]);
        let lo1 = _mm256_castps256_ps128(v1);
        let hi1 = _mm256_extractf128_ps::<1>(v1);
        let mut r1 = _mm_add_ps(lo1, hi1);
        r1 = _mm_add_ps(r1, _mm_movehl_ps(r1, r1));
        r1 = _mm_add_ss(r1, _mm_shuffle_ps::<0x55>(r1, r1));
        let mut a = _mm_cvtss_f32(r1);
        let lo2 = _mm256_castps256_ps128(v2);
        let hi2 = _mm256_extractf128_ps::<1>(v2);
        let mut r2 = _mm_add_ps(lo2, hi2);
        r2 = _mm_add_ps(r2, _mm_movehl_ps(r2, r2));
        r2 = _mm_add_ss(r2, _mm_shuffle_ps::<0x55>(r2, r2));
        let mut b = _mm_cvtss_f32(r2);
        while i < n {
            let ii = i as usize;
            a = MAC16_16(a, *x.get_unchecked(ii), *y01.get_unchecked(ii));
            b = MAC16_16(b, *x.get_unchecked(ii), *y02.get_unchecked(ii));
            i += 1;
        }
        *xy1 = a;
        *xy2 = b;
    }
}

/// `[fast]` 8-lane AVX2+FMA `xcorr_kernel`: four lags (`sum[0..4]`), eight `m`
/// values per iteration, one `__m256` FMA accumulator per lag.  Each
/// accumulator is reduced with the same `(s0+s4)+(s2+s6) + (s1+s5)+(s3+s7)`
/// tree as the inner-product variants, and the `len mod 8` tail is added with
/// scalar `MAC16_16` on top of the caller's initial `sum`.
///
/// # Slice lengths
/// Same contract as [`xcorr_kernel`]: `x.len() >= len`, `y.len() >= len + 3`.
/// The vector loop runs while `i + 8 <= len`, so its last `y` load starts at
/// `i + 3 <= len - 5` and its highest element is `y[len + 2]`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
#[target_feature(enable = "avx2,fma")]
unsafe fn xcorr_kernel_avx2_fma_hw(x: &[f32], y: &[f32], sum: &mut [f32; 4], len: i32) {
    use core::arch::x86_64::*;
    debug_assert!(len >= 8);
    debug_assert!(x.len() >= len as usize);
    debug_assert!(y.len() >= len as usize + 3);
    // SAFETY: the vector loop's loads are `x[i .. i+8]` with `i+8 <= len` and
    // `y[i+k .. i+k+8]` with `k <= 3`, whose highest index is `len+2`; the tail
    // reads `y[i+3]` with `i < len`.  `sum` is a `[f32; 4]`.
    unsafe {
        let mut s = [_mm256_setzero_ps(); 4];
        let mut i: i32 = 0;
        while i + 8 <= len {
            let ii = i as usize;
            let x0 = _mm256_loadu_ps(x.as_ptr().add(ii));
            s[0] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii)), s[0]);
            s[1] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 1)), s[1]);
            s[2] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 2)), s[2]);
            s[3] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 3)), s[3]);
            i += 8;
        }
        let mut out = *sum;
        for k in 0..4 {
            let lo = _mm256_castps256_ps128(s[k]);
            let hi = _mm256_extractf128_ps::<1>(s[k]);
            let mut r = _mm_add_ps(lo, hi);
            r = _mm_add_ps(r, _mm_movehl_ps(r, r));
            r = _mm_add_ss(r, _mm_shuffle_ps::<0x55>(r, r));
            out[k] += _mm_cvtss_f32(r);
        }
        while i < len {
            let ii = i as usize;
            let xv = *x.get_unchecked(ii);
            out[0] = MAC16_16(out[0], xv, *y.get_unchecked(ii));
            out[1] = MAC16_16(out[1], xv, *y.get_unchecked(ii + 1));
            out[2] = MAC16_16(out[2], xv, *y.get_unchecked(ii + 2));
            out[3] = MAC16_16(out[3], xv, *y.get_unchecked(ii + 3));
            i += 1;
        }
        *sum = out;
    }
}

/// `celt/x86/pitch_sse.c:108` `celt_inner_prod_sse`.
///
/// One 4-lane accumulator; the horizontal reduction is exactly
/// `(s0 + s2) + (s1 + s3)`, then the remaining `N mod 4` elements are added
/// sequentially with `MAC16_16` (mul then add, no FMA).
///
/// With `--features fast` and AVX2+FMA present, an 8-lane FMA variant
/// ([`celt_inner_prod_avx2_fma_hw`]) is used for `N >= 32` instead.  It is
/// arithmetically close but not bit-identical, which is the point of the
/// feature.
///
/// `x.len() >= N` and `y.len() >= N` (no over-read).
pub fn celt_inner_prod(x: &[f32], y: &[f32], n: i32) -> f32 {
    debug_assert!(n >= 0);
    debug_assert!(x.len() >= n as usize);
    debug_assert!(y.len() >= n as usize);
    #[cfg(all(feature = "fast", target_arch = "x86_64"))]
    {
        /* `n >= 32`: below that the 8-lane reduction (extract, two shuffles,
           two adds) costs more than the vector loop saves; measured n=8 4.7 ->
           6.5 ns, n=16 5.9 -> 6.5 ns, n=32 8.3 -> 7.5 ns. */
        if n >= 32 && have_avx2_fma() {
            // SAFETY: `have_avx2_fma()` guarantees the instruction set and the
            // asserts above give the slice lengths.
            return unsafe { celt_inner_prod_avx2_fma_hw(x, y, n) };
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE/SSE2 are baseline; the asserts above give the length
        // contract of `celt_inner_prod_sse_hw`.
        unsafe { celt_inner_prod_sse_hw(x, y, n) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut sum = [0.0f32; 4];
        let mut i: i32 = 0;
        while i < n - 3 {
            let ii = i as usize;
            sum = v4_add(sum, v4_mul(v4_load(x, ii), v4_load(y, ii)));
            i += 4;
        }
        // _mm_add_ps(sum, _mm_movehl_ps(sum,sum)) then _mm_add_ss with
        // _mm_shuffle_ps(sum,sum,0x55): lane 0 becomes (s0+s2) + (s1+s3).
        let t = v4_add(sum, [sum[2], sum[3], sum[2], sum[3]]);
        let mut xy = t[0] + t[1];
        while i < n {
            let ii = i as usize;
            xy = MAC16_16(xy, x[ii], y[ii]);
            i += 1;
        }
        xy
    }
}

/// `celt/pitch.h:159` `celt_inner_prod_c` — plain sequential C reference.
pub fn celt_inner_prod_c(x: &[f32], y: &[f32], n: i32) -> f32 {
    debug_assert!(n >= 0);
    let mut xy = 0.0f32;
    let mut i = 0usize;
    while i < n as usize {
        xy = MAC16_16(xy, x[i], y[i]);
        i += 1;
    }
    xy
}

/// `celt/x86/pitch_sse.c:79` `dual_inner_prod_sse`, transcribed with
/// `core::arch::x86_64` intrinsics (see [`celt_inner_prod_sse_hw`]).
///
/// `x.len() >= n >= 0`, `y01.len() >= n`, `y02.len() >= n`.
#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn dual_inner_prod_sse_hw(
    x: &[f32],
    y01: &[f32],
    y02: &[f32],
    n: i32,
    xy1: &mut f32,
    xy2: &mut f32,
) {
    use core::arch::x86_64::*;

    // SAFETY: the vector loop runs while `i < n - 3` and loads
    // `x[i..i+4]`, `y01[i..i+4]`, `y02[i..i+4]`, all in bounds by the caller's
    // contract; the tail is scalar and `i < n` there.
    unsafe {
        let mut xsum1 = _mm_setzero_ps();
        let mut xsum2 = _mm_setzero_ps();
        let mut i: i32 = 0;
        while i < n - 3 {
            let ii = i as usize;
            let xi = _mm_loadu_ps(x.as_ptr().add(ii));
            xsum1 = _mm_add_ps(
                xsum1,
                _mm_mul_ps(xi, _mm_loadu_ps(y01.as_ptr().add(ii))),
            );
            xsum2 = _mm_add_ps(
                xsum2,
                _mm_mul_ps(xi, _mm_loadu_ps(y02.as_ptr().add(ii))),
            );
            i += 4;
        }
        /* Horizontal sum, one accumulator at a time, before the tail loop --
           exactly the C order (`_mm_store_ss` both, then the scalar loop). */
        xsum1 = _mm_add_ps(xsum1, _mm_movehl_ps(xsum1, xsum1));
        xsum1 = _mm_add_ss(xsum1, _mm_shuffle_ps::<0x55>(xsum1, xsum1));
        xsum2 = _mm_add_ps(xsum2, _mm_movehl_ps(xsum2, xsum2));
        xsum2 = _mm_add_ss(xsum2, _mm_shuffle_ps::<0x55>(xsum2, xsum2));
        let mut a = _mm_cvtss_f32(xsum1);
        let mut b = _mm_cvtss_f32(xsum2);
        while i < n {
            let ii = i as usize;
            a = MAC16_16(a, *x.get_unchecked(ii), *y01.get_unchecked(ii));
            b = MAC16_16(b, *x.get_unchecked(ii), *y02.get_unchecked(ii));
            i += 1;
        }
        *xy1 = a;
        *xy2 = b;
    }
}

/// `celt/x86/pitch_sse.c:79` `dual_inner_prod_sse`.
///
/// Two independent 4-lane accumulators, each reduced as `(s0+s2)+(s1+s3)`,
/// then the remaining `N mod 4` elements are accumulated sequentially.
/// With `--features fast` and AVX2+FMA present, the 8-lane FMA variant
/// [`dual_inner_prod_avx2_fma_hw`] is used for `N >= 32` instead.
/// `x`, `y01` and `y02` need `N` elements each; `xy1`/`xy2` are written.
pub fn dual_inner_prod(
    x: &[f32],
    y01: &[f32],
    y02: &[f32],
    n: i32,
    xy1: &mut f32,
    xy2: &mut f32,
) {
    debug_assert!(n >= 0);
    debug_assert!(x.len() >= n as usize);
    debug_assert!(y01.len() >= n as usize);
    debug_assert!(y02.len() >= n as usize);
    #[cfg(all(feature = "fast", target_arch = "x86_64"))]
    {
        if n >= 32 && have_avx2_fma() {
            // SAFETY: `have_avx2_fma()` guarantees the instruction set and the
            // asserts above give the slice lengths.
            unsafe { dual_inner_prod_avx2_fma_hw(x, y01, y02, n, xy1, xy2) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE/SSE2 are baseline; the asserts above give the length
        // contract of `dual_inner_prod_sse_hw`.
        unsafe { dual_inner_prod_sse_hw(x, y01, y02, n, xy1, xy2) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut xsum1 = [0.0f32; 4];
        let mut xsum2 = [0.0f32; 4];
        let mut i: i32 = 0;
        while i < n - 3 {
            let ii = i as usize;
            let xi = v4_load(x, ii);
            xsum1 = v4_add(xsum1, v4_mul(xi, v4_load(y01, ii)));
            xsum2 = v4_add(xsum2, v4_mul(xi, v4_load(y02, ii)));
            i += 4;
        }
        let t1 = v4_add(xsum1, [xsum1[2], xsum1[3], xsum1[2], xsum1[3]]);
        let t2 = v4_add(xsum2, [xsum2[2], xsum2[3], xsum2[2], xsum2[3]]);
        let mut a = t1[0] + t1[1];
        let mut b = t2[0] + t2[1];
        while i < n {
            let ii = i as usize;
            a = MAC16_16(a, x[ii], y01[ii]);
            b = MAC16_16(b, x[ii], y02[ii]);
            i += 1;
        }
        *xy1 = a;
        *xy2 = b;
    }
}

/// `celt/pitch.h:137` `dual_inner_prod_c` — plain sequential C reference.
pub fn dual_inner_prod_c(
    x: &[f32],
    y01: &[f32],
    y02: &[f32],
    n: i32,
    xy1: &mut f32,
    xy2: &mut f32,
) {
    debug_assert!(n >= 0);
    let mut xy01 = 0.0f32;
    let mut xy02 = 0.0f32;
    let mut i = 0usize;
    while i < n as usize {
        xy01 = MAC16_16(xy01, x[i], y01[i]);
        xy02 = MAC16_16(xy02, x[i], y02[i]);
        i += 1;
    }
    *xy1 = xy01;
    *xy2 = xy02;
}

// ---------------------------------------------------------------------------
// comb_filter_const
// ---------------------------------------------------------------------------

/// `celt/x86/pitch_sse.c:133` `comb_filter_const_sse`.
///
/// Per 4-output block the last two terms are grouped `(a+b) + (c+d)` ("partial
/// sums", the active `#else` branch) instead of the left-to-right
/// `((a+b)+c)+d` of the C version, while the final `N mod 4` samples (the
/// active `CUSTOM_MODES` tail) use the plain C grouping.  One call therefore
/// mixes both associations.
///
/// # Index convention
/// The C signature is `comb_filter_const(y, x, T, N, ...)` and the kernel reads
/// `x[-T-2 .. )`.  Rust slices cannot be indexed negatively, so the `x` slice
/// passed here must start `T+2` elements *before* the C `x` pointer:
/// `x_rust[i] == x_c[i - T - 2]`, i.e. `x_c[i] == x_rust[i + T + 2]`.
/// All accesses then stay in bounds:
///
/// * the initial `x0v = _mm_loadu_ps(&x[-T-2])` is `x[0..4]`;
/// * the loop's `xp+4` is `x[i+4..i+8]` for the 4-block starting at output `i`;
/// * the tail reads `x[i ..= i+T+2]`.
///
/// Required lengths: `y.len() >= N` and `x.len() >= N + max(4, T + 2)`.
pub fn comb_filter_const(y: &mut [f32], x: &[f32], t: i32, n: i32, g10: f32, g11: f32, g12: f32) {
    debug_assert!(t >= 0);
    debug_assert!(n >= 0);
    debug_assert!(y.len() >= n as usize);
    debug_assert!(
        x.len() >= n as usize + 4.max(t as usize + 2),
        "comb_filter_const: x needs N + max(4, T+2) elements"
    );
    /* SSE is the x86-64 baseline (`OPUS_X86_PRESUME_SSE` in the reference
       build), so the C body of `comb_filter_const_sse` is transcribed
       instruction for instruction — including the `#else` "partial sums"
       grouping `(g11·(x3+x1)) + (g12·(x4+x0))`, which is what makes it differ
       from `comb_filter_const_c`.  The `[f32; 4]` emulation below stays as the
       fallback. */
    /* `--features fast` deliberately keeps this body: an `_mm_fmadd_ps`
       variant was measured 460 -> 475 ns per N=960 call (+3 %) — the kernel is
       load/shuffle-bound and the out-of-line `target_feature` call costs more
       than the three fused multiply-adds save — so it was reverted. */
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        /* SAFETY: SSE is the x86-64 baseline, and the asserts above bound every
           access: the loop's loads are `x[i+T+2 .. i+T+6]` and `x[i+4 .. i+8]`
           for `i <= n-4` (all inside `n + T + 2` / `n + 4`), `x0v` starts at
           `x[0..4]`, and the store writes `y[i .. i+4]` for `i <= n-4`. */
        unsafe {
            let g10v = _mm_load1_ps(&g10);
            let g11v = _mm_load1_ps(&g11);
            let g12v = _mm_load1_ps(&g12);
            let mut x0v = _mm_loadu_ps(x.as_ptr());
            let mut i: i32 = 0;
            while i < n - 3 {
                let ii = i as usize;
                /* yi = _mm_loadu_ps(x+i): the current output's direct term,
                   i.e. x_c[i..i+4] == x_rust[i+T+2 .. i+T+6]. */
                let yi = _mm_loadu_ps(x.as_ptr().add(ii + t as usize + 2));
                let x4v = _mm_loadu_ps(x.as_ptr().add(ii + 4));
                let x2v = _mm_shuffle_ps(x0v, x4v, 0x4e);
                let x1v = _mm_shuffle_ps(x0v, x2v, 0x99);
                let x3v = _mm_shuffle_ps(x2v, x4v, 0x99);

                let mut yi = _mm_add_ps(yi, _mm_mul_ps(g10v, x2v));
                /* Use partial sums (`#else` branch of `#if 0`). */
                let yi2 = _mm_add_ps(
                    _mm_mul_ps(g11v, _mm_add_ps(x3v, x1v)),
                    _mm_mul_ps(g12v, _mm_add_ps(x4v, x0v)),
                );
                yi = _mm_add_ps(yi, yi2);
                x0v = x4v;
                _mm_storeu_ps(y.as_mut_ptr().add(ii), yi);
                i += 4;
            }
            comb_filter_const_tail(y, x, t as usize, n, i, g10, g11, g12);
        }
        return;
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let g10v = splat4(g10);
        let g11v = splat4(g11);
        let g12v = splat4(g12);
        let mut x0v = v4_load(x, 0);
        let mut i: i32 = 0;
        while i < n - 3 {
            let ii = i as usize;
            // yi = _mm_loadu_ps(x+i): the current output's direct term, i.e.
            // x_c[i..i+4] == x_rust[i+T+2 .. i+T+6].
            let yi = v4_load(x, ii + t as usize + 2);
            let x4v = v4_load(x, ii + 4);
            let x2v = shuffle_ps(x0v, x4v, 0x4e);
            let x1v = shuffle_ps(x0v, x2v, 0x99);
            let x3v = shuffle_ps(x2v, x4v, 0x99);

            let mut yi = v4_add(yi, v4_mul(g10v, x2v));
            // Use partial sums
            let yi2 = v4_add(
                v4_mul(g11v, v4_add(x3v, x1v)),
                v4_mul(g12v, v4_add(x4v, x0v)),
            );
            yi = v4_add(yi, yi2);
            x0v = x4v;
            v4_store(y, ii, yi);
            i += 4;
        }
        comb_filter_const_tail(y, x, t as usize, n, i, g10, g11, g12);
    }
}

/// `comb_filter_const_sse`'s `CUSTOM_MODES` tail: plain left-to-right C
/// grouping
/// `y[i] = x_c[i] + g10·x_c[i-T] + g11·(x_c[i-T+1]+x_c[i-T-1])
///                + g12·(x_c[i-T+2]+x_c[i-T-2])`
/// with `x_c[i] == x[i+T+2]`.
#[inline]
#[allow(clippy::too_many_arguments)]
fn comb_filter_const_tail(
    y: &mut [f32],
    x: &[f32],
    tt: usize,
    n: i32,
    from: i32,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    let mut i = from;
    while i < n {
        let ii = i as usize;
        let v = x[ii + tt + 2]
            + g10 * x[ii + 2]
            + g11 * (x[ii + 3] + x[ii + 1])
            + g12 * (x[ii + 4] + x[ii]);
        y[ii] = v;
        i += 1;
    }
}

/// `celt/celt.c:166` `comb_filter_const_c` — the plain-C grouping
/// (`((x[n] + g10·x[n-T]) + g11·(…)) + g12·(…)` at every sample).  Not compiled
/// in the reference configuration (`NON_STATIC_COMB_FILTER_CONST_C` is undefined
/// and `OVERRIDE_COMB_FILTER_CONST` is set); kept for the probe.
/// Same index convention as [`comb_filter_const`].
pub fn comb_filter_const_c(y: &mut [f32], x: &[f32], t: i32, n: i32, g10: f32, g11: f32, g12: f32) {
    debug_assert!(n >= 0);
    debug_assert!(y.len() >= n as usize);
    debug_assert!(x.len() >= n as usize + 4.max(t as usize + 2));
    let tt = t as usize;
    // x4 = x_c[-T-2] ... x1 = x_c[-T+1]
    let mut x4 = x[0];
    let mut x3 = x[1];
    let mut x2 = x[2];
    let mut x1 = x[3];
    let mut i = 0usize;
    while i < n as usize {
        // x0 = x_c[i-T+2] == x_rust[i+4]
        let x0 = x[i + 4];
        y[i] = x[i + tt + 2] + g10 * x2 + g11 * (x1 + x3) + g12 * (x0 + x4);
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// celt_fir
// ---------------------------------------------------------------------------

/// `celt/celt_lpc.c:146` `celt_fir_c` (float build).
///
/// The float build has no `celt_fir_sse4_1` (`#if defined(FIXED_POINT)` only,
/// and the symbol is absent from the reference library), so this is the live
/// implementation.  It calls `xcorr_kernel` — `xcorr_kernel_sse` here — to
/// compute four taps at a time, initialising the accumulators with `x[i..i+4]`
/// (`SHL32(EXTEND32(x[i]), SIG_SHIFT)` is `x[i]` in the float build) and
/// rounding with `SROUND16` (an identity here).  The final `N mod 4` outputs
/// use the scalar `j = 0..ord` MAC16_16 loop.
///
/// # Index convention
/// The C reads `x[i+j-ord]`, so the `x` slice must start `ord` elements before
/// the C `x`: `x_rust[i] == x_c[i - ord]`, i.e. `x_c[k] == x_rust[k + ord]`.
/// Required lengths: `x.len() >= N + ord` (the `xcorr_kernel` call reads 3
/// elements past `ord`), `num.len() >= ord`, `y.len() >= N`.
pub fn celt_fir(x: &[f32], num: &[f32], y: &mut [f32], n: i32, ord: i32) {
    debug_assert!(ord > 0);
    debug_assert!(n >= 0);
    let nn = n as usize;
    let oo = ord as usize;
    debug_assert!(num.len() >= oo);
    debug_assert!(y.len() >= nn);
    debug_assert!(x.len() >= nn + oo, "celt_fir: x needs N + ord elements");
    let mut rnum_storage = [0.0f32; CELT_LPC_ORDER as usize];
    let rnum = &mut rnum_storage[..oo];
    /* `for (i=0;i<ord;i++) rnum[i] = num[ord-i-1];` — reversed copy without a
       per-element bounds check (the lengths are equal by construction). */
    for (r, tap) in rnum.iter_mut().zip(num[..oo].iter().rev()) {
        *r = *tap;
    }
    let mut i: i32 = 0;
    while i < n - 3 {
        let ii = i as usize;
        // sum[k] = SHL32(EXTEND32(x[i+k]), SIG_SHIFT) == x_c[i+k]
        let mut sum = [
            x[ii + oo],
            x[ii + oo + 1],
            x[ii + oo + 2],
            x[ii + oo + 3],
        ];
        // xcorr_kernel(rnum, x_c+i-ord, sum, ord) with x_c[i-ord] == x_rust[i].
        xcorr_kernel(&rnum, &x[ii..], &mut sum, ord);
        v4_store(y, ii, sum);
        i += 4;
    }
    while i < n {
        let ii = i as usize;
        let mut sum = x[ii + oo];
        for j in 0..oo {
            sum = MAC16_16(sum, rnum[j], x[ii + j]);
        }
        y[ii] = sum;
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// celt_pitch_xcorr
// ---------------------------------------------------------------------------

/// `celt/x86/pitch_avx.c:39` `xcorr_kernel_avx` (static in the C).
///
/// Eight `__m256` (8-lane) accumulators, each updated with a true FMA.  The
/// final reduction is the C sequence
/// `permute2f128(0x20) + permute2f128(0x31)` → `hadd` → `hadd`, which for
/// output `p` works out to `((s0+s4)+(s1+s5)) + ((s2+s6)+(s3+s7))` with `s_k`
/// lane `k` of accumulator `p`.
///
/// `x.len() >= len`, `y.len() >= len + 7`: the main loop loads 8 floats at
/// `y+i+k` for `k = 0..8`, up to `y[len+6]`.
/// The body, factored out so that it can be inlined both into the
/// AVX2+FMA build (where `f32::mul_add` becomes a single `vfmadd`
/// instruction, as in the C) and into the portable fallback (where it stays a
/// call to compiler-builtins' correctly-rounded `fmaf`).
///
/// `#[inline(always)]`: the caller decides the instruction set.  Inlining a
/// feature-poorer callee into an AVX2+FMA caller is what lets
/// `xcorr_kernel_avx_hw` below reach the FMA instruction at all.
#[inline(always)]
fn xcorr_kernel_avx_body(x: &[f32], y: &[f32], sum: &mut [f32; 8], len: i32) {
    debug_assert!(len > 0);
    debug_assert!(x.len() >= len as usize);
    debug_assert!(
        y.len() >= len as usize + 7,
        "xcorr_kernel_avx: y needs len+7 elements"
    );
    let mut s = [[0.0f32; 8]; 8];
    let mut i: i32 = 0;
    while i < len - 7 {
        let ii = i as usize;
        let x0 = v8_load(x, ii);
        s[0] = v8_fmadd(x0, v8_load(y, ii), s[0]);
        s[1] = v8_fmadd(x0, v8_load(y, ii + 1), s[1]);
        s[2] = v8_fmadd(x0, v8_load(y, ii + 2), s[2]);
        s[3] = v8_fmadd(x0, v8_load(y, ii + 3), s[3]);
        s[4] = v8_fmadd(x0, v8_load(y, ii + 4), s[4]);
        s[5] = v8_fmadd(x0, v8_load(y, ii + 5), s[5]);
        s[6] = v8_fmadd(x0, v8_load(y, ii + 6), s[6]);
        s[7] = v8_fmadd(x0, v8_load(y, ii + 7), s[7]);
        i += 8;
    }
    if i != len {
        // `mask + 7 + i - len` selects exactly the remaining r = len-i lanes;
        // _mm256_maskload_ps zeroes the masked-out elements (so those lanes
        // contribute fma(0,0,acc)).
        let r = (len - i) as usize;
        debug_assert!(r >= 1 && r <= 7);
        let ii = i as usize;
        let mut x0 = [0.0f32; 8];
        for l in 0..r {
            x0[l] = x[ii + l];
        }
        for k in 0..8 {
            let mut yv = [0.0f32; 8];
            for l in 0..r {
                yv[l] = y[ii + k + l];
            }
            s[k] = v8_fmadd(x0, yv, s[k]);
        }
    }
    // 8 horizontal adds:
    //   xsum_p = permute2f128(s_p, s_{p+4}, 2<<4) + permute2f128(s_p, s_{p+4}, 1|(3<<4))
    let a0 = v8_add(permute2f128(s[0], s[4], 2 << 4), permute2f128(s[0], s[4], 1 | (3 << 4)));
    let a1 = v8_add(permute2f128(s[1], s[5], 2 << 4), permute2f128(s[1], s[5], 1 | (3 << 4)));
    let a2 = v8_add(permute2f128(s[2], s[6], 2 << 4), permute2f128(s[2], s[6], 1 | (3 << 4)));
    let a3 = v8_add(permute2f128(s[3], s[7], 2 << 4), permute2f128(s[3], s[7], 1 | (3 << 4)));
    let b0 = hadd8(a0, a1);
    let b1 = hadd8(a2, a3);
    *sum = hadd8(b0, b1);
}

/// True when the host can execute the AVX2+FMA form of these kernels.
///
/// The reference configuration is compiled for the x86-64 baseline (`no -mfma`,
/// no `.cargo/config` in the port), so without this dispatch `f32::mul_add`
/// stays a call to compiler-builtins' `fmaf` — correct but ~5x the cost of the
/// `vfmadd` the C emits.  `is_x86_feature_detected!` caches its answer.
/// (Shared with `silk/float/x86/inner_product_FLP_avx2`, the other FMA user.)
#[inline(always)]
pub(crate) fn have_avx2_fma() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// [`xcorr_kernel_avx`] compiled for AVX2+FMA: the C body transcribed with
/// `core::arch::x86_64` intrinsics.
///
/// The portable `[f32; 8]` emulation in [`xcorr_kernel_avx_body`] is
/// arithmetically identical, but LLVM does **not** vectorise it: inlining it
/// into `celt_pitch_xcorr_hw` produces scalar code (measured on this build:
/// 66 `vfmadd231ss`, 486 `vmovss`, only 8 `ymm` references), i.e. about one
/// multiply-add per instruction.  `run_prefilter`'s coarse pitch search then
/// costs ~50 us/frame instead of the ~5 us the packed C kernel needs, and the
/// pitch search is ~67 % of a CELT-only encode frame.  Writing the eight
/// `__m256` accumulators and the `permute2f128`/`hadd` reduction with the
/// actual intrinsics reproduces `celt/x86/pitch_avx.c:39` instruction for
/// instruction (same FMA order, same lane layout), so the result is unchanged.
///
/// `x.len() >= len`, `y.len() >= len + 7`: the masked tail reads a full 8-lane
/// window at `y[len - r + k]`, `k = 0..8`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn xcorr_kernel_avx_hw(x: &[f32], y: &[f32], sum: &mut [f32; 8], len: i32) {
    use core::arch::x86_64::*;

    debug_assert!(len > 0);
    debug_assert!(x.len() >= len as usize);
    debug_assert!(
        y.len() >= len as usize + 7,
        "xcorr_kernel_avx: y needs len+7 elements"
    );

    // SAFETY: `xcorr_kernel_avx_hw` is only called behind `have_avx2_fma()`,
    // which is the feature requirement of every intrinsic below.  All loads
    // stay inside the slices: the vector loop runs while `i < len - 7`, so
    // `x[i..i+8]` and `y[i+k..i+k+8]` (k <= 7) end at `y[i+15] <= y[len+7]`
    // (contract above), and the masked tail loads the 8-lane window at
    // `y[len-r..len-r+8]` with `r = len - i in 1..=7`, i.e. within `y[len+7]`.
    unsafe {
        let mut s = [_mm256_setzero_ps(); 8];
        let mut i: i32 = 0;
        while i < len - 7 {
            let ii = i as usize;
            let x0 = _mm256_loadu_ps(x.as_ptr().add(ii));
            s[0] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii)), s[0]);
            s[1] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 1)), s[1]);
            s[2] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 2)), s[2]);
            s[3] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 3)), s[3]);
            s[4] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 4)), s[4]);
            s[5] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 5)), s[5]);
            s[6] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 6)), s[6]);
            s[7] = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y.as_ptr().add(ii + 7)), s[7]);
            i += 8;
        }
        if i != len {
            /* The C does
               `m = _mm256_loadu_si256((__m256i*)(mask + 7 + i - len));`
               with `mask = {7 x -1, 8 x 0}`, i.e. the low `len - i` lanes are
               active; `_mm256_maskload_ps` zeroes the masked-out elements, so
               those lanes contribute `fma(0, 0, acc) == acc`. */
            let r = len - i; /* 1..=7 */
            let m = _mm256_cmpgt_epi32(
                _mm256_set1_epi32(r),
                _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
            );
            let ii = i as usize;
            let x0 = _mm256_maskload_ps(x.as_ptr().add(ii), m);
            s[0] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii), m), s[0]);
            s[1] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 1), m), s[1]);
            s[2] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 2), m), s[2]);
            s[3] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 3), m), s[3]);
            s[4] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 4), m), s[4]);
            s[5] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 5), m), s[5]);
            s[6] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 6), m), s[6]);
            s[7] = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y.as_ptr().add(ii + 7), m), s[7]);
        }
        /* 8 horizontal adds, as in the C:
           permute2f128(0x20) + permute2f128(0x31) -> hadd -> hadd. */
        let a0 = _mm256_add_ps(
            _mm256_permute2f128_ps::<{ 2 << 4 }>(s[0], s[4]),
            _mm256_permute2f128_ps::<{ 1 | (3 << 4) }>(s[0], s[4]),
        );
        let a1 = _mm256_add_ps(
            _mm256_permute2f128_ps::<{ 2 << 4 }>(s[1], s[5]),
            _mm256_permute2f128_ps::<{ 1 | (3 << 4) }>(s[1], s[5]),
        );
        let a2 = _mm256_add_ps(
            _mm256_permute2f128_ps::<{ 2 << 4 }>(s[2], s[6]),
            _mm256_permute2f128_ps::<{ 1 | (3 << 4) }>(s[2], s[6]),
        );
        let a3 = _mm256_add_ps(
            _mm256_permute2f128_ps::<{ 2 << 4 }>(s[3], s[7]),
            _mm256_permute2f128_ps::<{ 1 | (3 << 4) }>(s[3], s[7]),
        );
        let b0 = _mm256_hadd_ps(a0, a1);
        let b1 = _mm256_hadd_ps(a2, a3);
        _mm256_storeu_ps(sum.as_mut_ptr(), _mm256_hadd_ps(b0, b1));
    }
}

/// `celt/x86/pitch_avx.c:39` `xcorr_kernel_avx` (static in the C), dispatched at
/// run time exactly like the C's RTCD selection.
fn xcorr_kernel_avx(x: &[f32], y: &[f32], sum: &mut [f32; 8], len: i32) {
    #[cfg(target_arch = "x86_64")]
    if have_avx2_fma() {
        // SAFETY: guarded by `have_avx2_fma()`.
        unsafe { xcorr_kernel_avx_hw(x, y, sum, len) };
        return;
    }
    xcorr_kernel_avx_body(x, y, sum, len)
}

/// `celt/x86/pitch_avx.c:86` `celt_pitch_xcorr_avx2` (the `PITCH_XCORR_IMPL[4]`
/// entry).  Writes `max_pitch` outputs, `xcorr[i] = Σ x[m]·y[i+m]`:
/// 8 lags at a time with [`xcorr_kernel_avx`], the remaining `max_pitch mod 8`
/// lags one at a time with the SSE [`celt_inner_prod`].
///
/// `x.len() >= len`, `xcorr.len() >= max_pitch` and
/// `y.len() >= max_pitch + len - 1` (the AVX2 blocks read up to
/// `y[max_pitch-8+14]`, the SSE tail up to `y[max_pitch-1+len-1]`).
pub fn celt_pitch_xcorr(x: &[f32], y: &[f32], xcorr: &mut [f32], len: i32, max_pitch: i32) {
    #[cfg(all(feature = "fast", target_arch = "x86_64"))]
    if have_avx2_fma() {
        // SAFETY: guarded by `have_avx2_fma()`.
        unsafe {
            celt_pitch_xcorr_fast_hw(x, y, xcorr, len, max_pitch);
        }
        return;
    }
    #[cfg(target_arch = "x86_64")]
    if have_avx2_fma() {
        // SAFETY: guarded by `have_avx2_fma()`.
        unsafe {
            celt_pitch_xcorr_hw(x, y, xcorr, len, max_pitch);
        }
        return;
    }
    celt_pitch_xcorr_avx2_body(x, y, xcorr, len, max_pitch)
}

/// `[fast]` `celt_pitch_xcorr` with the 8-lag AVX2+FMA kernel called directly.
///
/// The arithmetic is identical to [`celt_pitch_xcorr_hw`] (the C already
/// contracts `xcorr_kernel_avx` into `celt_pitch_xcorr_avx2`); the difference is
/// that the per-8-lag-block `have_avx2_fma()` dispatch of [`xcorr_kernel_avx`]
/// is skipped, which matters for the small frame sizes where `celt_pitch_xcorr`
/// is called with only a handful of lag blocks.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
#[target_feature(enable = "avx2,fma")]
unsafe fn celt_pitch_xcorr_fast_hw(
    x: &[f32],
    y: &[f32],
    xcorr: &mut [f32],
    len: i32,
    max_pitch: i32,
) {
    debug_assert!(max_pitch > 0);
    debug_assert!(len > 0);
    debug_assert!(x.len() >= len as usize);
    debug_assert!(xcorr.len() >= max_pitch as usize);
    debug_assert!(y.len() >= (max_pitch - 1 + len) as usize);
    // SAFETY: `have_avx2_fma()` guards the call site; the asserts above give
    // the slice contract of `xcorr_kernel_avx_hw`.
    unsafe {
        let mut i: i32 = 0;
        while i < max_pitch - 7 {
            let ii = i as usize;
            let mut sum = [0.0f32; 8];
            xcorr_kernel_avx_hw(x, &y[ii..], &mut sum, len);
            for k in 0..8 {
                *xcorr.get_unchecked_mut(ii + k) = sum[k];
            }
            i += 8;
        }
        while i < max_pitch {
            let ii = i as usize;
            *xcorr.get_unchecked_mut(ii) = celt_inner_prod(x, &y[ii..], len);
            i += 1;
        }
    }
}

/// [`celt_pitch_xcorr_avx2_body`] compiled for AVX2+FMA.
///
/// The C compiles this function and its `static xcorr_kernel_avx` in the same
/// `-mavx2 -mfma` translation unit, so the 8-lag kernel is inlined and uses
/// packed `vfmadd`.  Here the kernel is the dispatching [`xcorr_kernel_avx`],
/// whose AVX2+FMA arm is the intrinsic [`xcorr_kernel_avx_hw`]; this wrapper
/// exists so that arm is reachable (an earlier version called
/// `xcorr_kernel_avx_body` directly, which LLVM scalarised).
///
/// Bit-exactness: AVX2/FMA change neither the rounding (FMA keeps its single
/// rounding) nor the operation order of the transliterated C.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn celt_pitch_xcorr_hw(x: &[f32], y: &[f32], xcorr: &mut [f32], len: i32, max_pitch: i32) {
    celt_pitch_xcorr_avx2_body(x, y, xcorr, len, max_pitch)
}

/// The body of the AVX2 `celt_pitch_xcorr`, shared by the hardware-FMA path and
/// the portable fallback: 8 lags at a time with [`xcorr_kernel_avx`] (the C's
/// `static xcorr_kernel_avx`, which the C inlines into `celt_pitch_xcorr_avx2`),
/// then the remaining `max_pitch mod 8` lags one at a time.
#[inline(always)]
fn celt_pitch_xcorr_avx2_body(x: &[f32], y: &[f32], xcorr: &mut [f32], len: i32, max_pitch: i32) {
    debug_assert!(max_pitch > 0);
    debug_assert!(len > 0);
    debug_assert!(x.len() >= len as usize);
    debug_assert!(xcorr.len() >= max_pitch as usize);
    debug_assert!(y.len() >= (max_pitch - 1 + len) as usize);
    let mut i: i32 = 0;
    while i < max_pitch - 7 {
        let ii = i as usize;
        let mut sum = [0.0f32; 8];
        xcorr_kernel_avx(x, &y[ii..], &mut sum, len);
        for k in 0..8 {
            xcorr[ii + k] = sum[k];
        }
        i += 8;
    }
    while i < max_pitch {
        let ii = i as usize;
        xcorr[ii] = celt_inner_prod(x, &y[ii..], len);
        i += 1;
    }
}

/// `celt/pitch.c:230` `celt_pitch_xcorr_c` — the plain-C entry point selected
/// by `PITCH_XCORR_IMPL[arch]` for `arch <= 3`.  Note that in this
/// configuration its `xcorr_kernel`/`celt_inner_prod` macros still resolve to
/// the SSE kernels, so only the 4-way split (versus the AVX2 8-way + FMA)
/// differs.  Same slice requirements as [`celt_pitch_xcorr`].
pub fn celt_pitch_xcorr_c(x: &[f32], y: &[f32], xcorr: &mut [f32], len: i32, max_pitch: i32) {
    debug_assert!(max_pitch > 0);
    debug_assert!(len > 0);
    debug_assert!(x.len() >= len as usize);
    debug_assert!(xcorr.len() >= max_pitch as usize);
    debug_assert!(y.len() >= (max_pitch - 1 + len) as usize);
    let mut i: i32 = 0;
    while i < max_pitch - 3 {
        let ii = i as usize;
        let mut sum = [0.0f32; 4];
        xcorr_kernel(x, &y[ii..], &mut sum, len);
        xcorr[ii] = sum[0];
        xcorr[ii + 1] = sum[1];
        xcorr[ii + 2] = sum[2];
        xcorr[ii + 3] = sum[3];
        i += 4;
    }
    while i < max_pitch {
        let ii = i as usize;
        xcorr[ii] = celt_inner_prod(x, &y[ii..], len);
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// op_pvq_search
// ---------------------------------------------------------------------------

/// `celt/x86/vq_sse2.c:44` `op_pvq_search_sse2`.
///
/// `arch` is a no-op (`OPUS_X86_PRESUME_SSE2` wires the macro directly to this
/// kernel at every arch).  `x` is *not* modified (the C copies it into a local
/// `X` of `N+3` floats); the padding lanes of `iy` are written and must exist.
///
/// # Slice lengths
/// `x.len() >= N`, `iy.len() >= N + 3` (every 4-wide store writes up to
/// `iy[N+2]`; `alg_quant` allocates `ALLOC(iy, N+3, int)` for this reason).
///
/// # Exact arithmetic
/// `_mm_rcp_ps`/`_mm_rsqrt_ps` are the *approximate* hardware operations (see
/// [`simd_rcp_ps`]/[`simd_rsqrt_ps`]), the score is maximised directly (no
/// cross-multiplied ratio test), and ties are broken toward the **largest**
/// index (`_mm_max_epi16` over `pos`); all of which differs from
/// [`op_pvq_search_c`].
pub fn op_pvq_search_sse2(x: &[f32], iy: &mut [i32], k: i32, n: i32, _arch: i32) -> f32 {
    debug_assert!(n > 0);
    debug_assert!(k > 0);
    let nn = n as usize;
    debug_assert!(x.len() >= nn);
    debug_assert!(iy.len() >= nn + 3);
    /* `--features fast`: the specialisations of the `[fast]` section below
       take `K == 1`, `N == 2` and `N == 4`, and otherwise run this body with
       scratch that is not zero-filled. */
    #[cfg(all(feature = "fast", target_arch = "x86_64"))]
    {
        if let Some(yy) = op_pvq_search_specialised(x, iy, k, nn) {
            return yy;
        }
    }
    // `VARDECL(celt_norm, y/X/signy); ALLOC(..., N+3, ...)` (`celt/x86/
    // vq_sse2.c`) — a real PVQ band is capped at `MAX_PVQ_N` by the mode
    // creator, so the codec path uses stack storage of the C size.  The
    // `kernels` probe calls this directly with `N` up to 2000, which no mode
    // can reach; only those cases fall back to the heap.
    let mut xx_heap: Vec<f32> = Vec::new();
    let mut y_heap: Vec<f32> = Vec::new();
    let mut signy_heap: Vec<f32> = Vec::new();
    let mut xx_storage = [0.0f32; MAX_PVQ_N + 3];
    let mut y_storage = [0.0f32; MAX_PVQ_N + 3];
    let mut signy_storage = [0.0f32; MAX_PVQ_N + 3];
    let xx: &mut [f32] = if nn + 3 <= MAX_PVQ_N + 3 {
        &mut xx_storage[..nn + 3]
    } else {
        xx_heap.resize(nn + 3, 0.0);
        xx_heap.as_mut_slice()
    };
    xx[..nn].copy_from_slice(&x[..nn]);
    let y: &mut [f32] = if nn + 3 <= MAX_PVQ_N + 3 {
        &mut y_storage[..nn + 3]
    } else {
        y_heap.resize(nn + 3, 0.0);
        y_heap.as_mut_slice()
    };
    let signy: &mut [f32] = if nn + 3 <= MAX_PVQ_N + 3 {
        &mut signy_storage[..nn + 3]
    } else {
        signy_heap.resize(nn + 3, 0.0);
        signy_heap.as_mut_slice()
    };


    /* `OPUS_X86_PRESUME_SSE2` wires the macro to `op_pvq_search_sse2` at every
       arch and SSE2 is part of the x86-64 baseline, so the C body is
       transcribed below with real intrinsics: one `_mm_storeu_si128` per four
       lanes (`v4_store`-style checked stores before), `_mm_shuffle_ps` /
       `_mm_shuffle_epi32` instead of lane-array shuffles, the hardware
       `_mm_rcp_ps` / `_mm_rsqrt_ps`, `_mm_cvttps_epi32`, and `_mm_max_epi16`
       for the index tie-break.  The portable lane emulation is kept for
       non-x86_64 targets. */
    #[cfg(target_arch = "x86_64")]
    {
        /* SAFETY: `xx`/`y`/`signy` each hold `nn + 3` floats and `iy` holds
           `nn + 3` ints (the C `ALLOC(..., N+3, ...)` contract), and SSE2 is
           the x86-64 baseline. */
        return unsafe {
            op_pvq_search_sse2_hw(xx.as_mut_ptr(), y.as_mut_ptr(), signy.as_mut_ptr(), iy, k, n)
        };
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        return op_pvq_search_sse2_emu(xx, y, signy, iy, k, n);
    }
}


/// The portable `[f32; 4]` lane emulation of [`op_pvq_search_sse2`], kept for
/// targets without SSE2.  On x86_64 the intrinsics transcription is used.
#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::too_many_arguments)]
fn op_pvq_search_sse2_emu(
    mut xx: &mut [f32],
    mut y: &mut [f32],
    mut signy: &mut [f32],
    iy: &mut [i32],
    k: i32,
    n: i32,
) -> f32 {
    let nn = n as usize;
    // Get rid of the sign
    let mut sums = [0.0f32; 4];
    let mut j = 0usize;
    while j < nn {
        let x4 = v4_load(&xx, j);
        let s4 = cmplt_ps_mask(x4, [0.0f32; 4]);
        let x4 = v4_abs(x4);
        sums = v4_add(sums, x4);
        // Clear y and iy in case we don't do the projection.
        v4_store(&mut y, j, [0.0f32; 4]);
        iy[j] = 0;
        iy[j + 1] = 0;
        iy[j + 2] = 0;
        iy[j + 3] = 0;
        v4_store(&mut xx, j, x4);
        signy[j] = f32::from_bits(s4[0] as u32);
        signy[j + 1] = f32::from_bits(s4[1] as u32);
        signy[j + 2] = f32::from_bits(s4[2] as u32);
        signy[j + 3] = f32::from_bits(s4[3] as u32);
        j += 4;
    }
    // _mm_shuffle_ps(sums,sums,_MM_SHUFFLE(1,0,3,2)) = 0x4E,
    // then _MM_SHUFFLE(2,3,0,1) = 0xB1; lane 0 is ((s0+s2)+(s1+s3)).
    sums = v4_add(sums, shuffle_ps(sums, sums, 0x4e));
    sums = v4_add(sums, shuffle_ps(sums, sums, 0xb1));

    let mut xy = 0.0f32;
    let mut yy = 0.0f32;
    let mut pulses_left = k;

    // Do a pre-search by projecting on the pyramid
    if k > (n >> 1) {
        let sum = sums[0];
        // If X is too small, just replace it with a pulse at 0.  Prevents
        // infinities and NaNs from causing too many pulses to be allocated.
        if !(sum > EPSILON && sum < 64.0) {
            xx[0] = QCONST16(1.0, 14);
            let mut j = 1usize;
            while j < nn {
                xx[j] = 0.0;
                j += 1;
            }
            sums = splat4(1.0);
        }
        // _mm_set_ps1((float)(K+.8)): the sum is computed in double, as in C.
        let rcp4 = v4_mul(splat4((k as f64 + 0.8) as f32), simd_rcp_ps_v4(sums));
        let mut xy4 = [0.0f32; 4];
        let mut yy4 = [0.0f32; 4];
        let mut pulses_sum = [0i32; 4];
        let mut j = 0usize;
        while j < nn {
            // Load _mm_loadu_ps(&X[j]): the four lanes all get the same scalar
            // _mm_rcp_ps result, so rcp4 has four equal lanes.
            let x4 = v4_load(&xx, j);
            let rx4 = v4_mul(x4, rcp4);
            let iy4 = cvttps_epi32(rx4);
            pulses_sum = v4_add_epi32(pulses_sum, iy4);
            iy[j] = iy4[0];
            iy[j + 1] = iy4[1];
            iy[j + 2] = iy4[2];
            iy[j + 3] = iy4[3];
            let y4 = cvtepi32_ps(iy4);
            xy4 = v4_add(xy4, v4_mul(x4, y4));
            yy4 = v4_add(yy4, v4_mul(y4, y4));
            // double the y[] vector so we don't have to do it in the search loop
            v4_store(&mut y, j, v4_add(y4, y4));
            j += 4;
        }
        pulses_sum = v4_add_epi32(pulses_sum, shuffle_epi32(pulses_sum, 0x4e));
        pulses_sum = v4_add_epi32(pulses_sum, shuffle_epi32(pulses_sum, 0xb1));
        pulses_left -= pulses_sum[0];
        xy4 = v4_add(xy4, shuffle_ps(xy4, xy4, 0x4e));
        xy4 = v4_add(xy4, shuffle_ps(xy4, xy4, 0xb1));
        xy = xy4[0];
        yy4 = v4_add(yy4, shuffle_ps(yy4, yy4, 0x4e));
        yy4 = v4_add(yy4, shuffle_ps(yy4, yy4, 0xb1));
        yy = yy4[0];
    }
    xx[nn] = -100.0;
    xx[nn + 1] = -100.0;
    xx[nn + 2] = -100.0;
    y[nn] = 100.0;
    y[nn + 1] = 100.0;
    y[nn + 2] = 100.0;
    celt_sig_assert!(pulses_left >= 0);

    // This should never happen, but just in case it does (e.g. on silence)
    // we fill the first bin with pulses.
    if pulses_left > n + 3 {
        let tmp = pulses_left as f32;
        yy = MAC16_16(yy, tmp, tmp);
        yy = MAC16_16(yy, tmp, y[0]);
        iy[0] += pulses_left;
        pulses_left = 0;
    }

    let mut i = 0i32;
    while i < pulses_left {
        // The squared magnitude term gets added anyway, so we might as well
        // add it outside the loop.
        yy = ADD16(yy, 1.0);
        let xy4 = splat4(xy);
        let yy4 = splat4(yy);
        let mut max = [0.0f32; 4];
        let mut pos = [0i32; 4];
        let mut count = [0i32, 1, 2, 3];
        let fours = [4i32; 4];
        let mut j = 0usize;
        while j < nn {
            let mut x4 = v4_load(&xx, j);
            let mut y4 = v4_load(&y, j);
            x4 = v4_add(x4, xy4);
            y4 = v4_add(y4, yy4);
            y4 = simd_rsqrt_ps_v4(y4);
            let r4 = v4_mul(x4, y4);
            // Update the index of the max.
            let m = cmpgt_ps_mask(r4, max);
            pos = max_epi16(
                pos,
                [
                    count[0] & m[0],
                    count[1] & m[1],
                    count[2] & m[2],
                    count[3] & m[3],
                ],
            );
            // Update the max.
            max = max_ps(max, r4);
            // Update the indices (+4)
            count = v4_add_epi32(count, fours);
            j += 4;
        }
        // Horizontal max
        let mut max2 = max_ps(max, shuffle_ps(max, max, 0x4e));
        max2 = max_ps(max2, shuffle_ps(max2, max2, 0xb1));
        // Now that max2 contains the max at all positions, look at which
        // value(s) of the partial max is equal to the global max.
        let eq = cmpeq_ps_mask(max, max2);
        pos = [
            pos[0] & eq[0],
            pos[1] & eq[1],
            pos[2] & eq[2],
            pos[3] & eq[3],
        ];
        pos = max_epi16(pos, [pos[2], pos[3], pos[2], pos[3]]);
        // _mm_shufflelo_epi16(pos, _MM_SHUFFLE(1,0,3,2)) swaps the low two
        // 32-bit lanes.
        pos = max_epi16(pos, [pos[1], pos[0], pos[2], pos[3]]);
        let best_id = pos[0] as usize;

        // Updating the sums of the new pulse(s)
        xy = ADD32(xy, xx[best_id]);
        // We're multiplying y[j] by two so we don't have to do it here
        yy = ADD16(yy, y[best_id]);

        // Only now that we've made the final choice, update y/iy
        y[best_id] += 2.0;
        iy[best_id] += 1;
        i += 1;
    }

    // Put the original sign back
    let mut j = 0usize;
    while j < nn {
        let y4 = [iy[j], iy[j + 1], iy[j + 2], iy[j + 3]];
        let s4 = [
            signy[j].to_bits() as i32,
            signy[j + 1].to_bits() as i32,
            signy[j + 2].to_bits() as i32,
            signy[j + 3].to_bits() as i32,
        ];
        let out = [
            (y4[0].wrapping_add(s4[0])) ^ s4[0],
            (y4[1].wrapping_add(s4[1])) ^ s4[1],
            (y4[2].wrapping_add(s4[2])) ^ s4[2],
            (y4[3].wrapping_add(s4[3])) ^ s4[3],
        ];
        iy[j] = out[0];
        iy[j + 1] = out[1];
        iy[j + 2] = out[2];
        iy[j + 3] = out[3];
        j += 4;
    }
    yy}

/// The C body of [`op_pvq_search_sse2`] (`celt/x86/vq_sse2.c:44`), transcribed
/// with SSE2 intrinsics.
///
/// `xx` is the C local `X`, `y` and `signy` are its other two `VARDECL`
/// buffers, all `N+3` long; `iy` is the `N+3` output.  Nothing here changes the
/// operation order of the C: the two `_mm_shuffle_ps` reductions
/// (`0x4E` then `0xB1`), the `(K+.8) * _mm_rcp_ps(sums)` projection, the
/// "double `y`" convention, the `_mm_max_epi16` tie-break chain and the final
/// `_mm_xor_si128` sign restore are the same instructions.
///
/// This body is also the `fast` build's general path: an `_mm_fmadd_ps` version
/// of the projection dot products was measured **9-11 % slower** (the loop is a
/// serial accumulator chain; FMA latency 5 vs add latency 3, and recompiling
/// the whole kernel under `target_feature(fma)` costs more than the contraction
/// saves), so `--features fast` deliberately keeps this exact arithmetic and
/// only changes the *storage* (see [`op_pvq_search_fast_body`]).
#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
unsafe fn op_pvq_search_sse2_hw(
    xp: *mut f32,
    yp: *mut f32,
    sp: *mut f32,
    iy: &mut [i32],
    k: i32,
    n: i32,
) -> f32 {
    use core::arch::x86_64::*;
    let nn = n as usize;
    /* SAFETY: SSE2 is part of the x86-64 baseline.  The caller guarantees that
       `xp`, `yp` and `sp` each point at `nn + 3` writable floats (the C
       `ALLOC(..., N+3, ...)` size) and that `iy` has `nn + 3` elements; every
       4-wide load/store below starts at a `j < nn` that is a multiple of 4 and
       therefore touches at most indices `nn+2`. */
    unsafe {
        let ip = iy.as_mut_ptr();
        /* All bits set to zero, except for the sign bit. */
        let signmask = _mm_set_ps1(-0.0);
        let fours = _mm_set1_epi32(4);
        let zero = _mm_setzero_ps();

        let mut sums = _mm_setzero_ps();
        let mut j = 0usize;
        while j < nn {
            let mut x4 = _mm_loadu_ps(xp.add(j));
            let s4 = _mm_cmplt_ps(x4, zero);
            /* Get rid of the sign */
            x4 = _mm_andnot_ps(signmask, x4);
            sums = _mm_add_ps(sums, x4);
            /* Clear y and iy in case we don't do the projection. */
            _mm_storeu_ps(yp.add(j), zero);
            _mm_storeu_si128(ip.add(j) as *mut __m128i, _mm_setzero_si128());
            _mm_storeu_ps(xp.add(j), x4);
            _mm_storeu_ps(sp.add(j), s4);
            j += 4;
        }
        sums = _mm_add_ps(sums, _mm_shuffle_ps::<0x4e>(sums, sums));
        sums = _mm_add_ps(sums, _mm_shuffle_ps::<0xb1>(sums, sums));

        let mut xy: f32 = 0.0;
        let mut yy: f32 = 0.0;
        let mut pulses_left = k;

        /* Do a pre-search by projecting on the pyramid */
        if k > (n >> 1) {
            let sum = _mm_cvtss_f32(sums);
            /* If X is too small, just replace it with a pulse at 0.  Prevents
               infinities and NaNs from causing too many pulses to be
               allocated.  64 is an approximation of infinity here. */
            if !(sum > EPSILON && sum < 64.0) {
                *xp = QCONST16(1.0, 14);
                let mut j = 1usize;
                while j < nn {
                    *xp.add(j) = 0.0;
                    j += 1;
                }
                sums = _mm_set_ps1(1.0);
            }
            /* Using K+e with e < 1 guarantees we cannot get more than K pulses. */
            let rcp4 = _mm_mul_ps(_mm_set_ps1((k as f64 + 0.8) as f32), _mm_rcp_ps(sums));
            let mut xy4 = _mm_setzero_ps();
            let mut yy4 = _mm_setzero_ps();
            let mut pulses_sum = _mm_setzero_si128();
            let mut j = 0usize;
            while j < nn {
                let x4 = _mm_loadu_ps(xp.add(j));
                let rx4 = _mm_mul_ps(x4, rcp4);
                let iy4 = _mm_cvttps_epi32(rx4);
                pulses_sum = _mm_add_epi32(pulses_sum, iy4);
                _mm_storeu_si128(ip.add(j) as *mut __m128i, iy4);
                let y4 = _mm_cvtepi32_ps(iy4);
                xy4 = _mm_add_ps(xy4, _mm_mul_ps(x4, y4));
                yy4 = _mm_add_ps(yy4, _mm_mul_ps(y4, y4));
                /* double the y[] vector so we don't have to do it in the search loop. */
                _mm_storeu_ps(yp.add(j), _mm_add_ps(y4, y4));
                j += 4;
            }
            pulses_sum = _mm_add_epi32(pulses_sum, _mm_shuffle_epi32::<0x4e>(pulses_sum));
            pulses_sum = _mm_add_epi32(pulses_sum, _mm_shuffle_epi32::<0xb1>(pulses_sum));
            pulses_left -= _mm_cvtsi128_si32(pulses_sum);
            xy4 = _mm_add_ps(xy4, _mm_shuffle_ps::<0x4e>(xy4, xy4));
            xy4 = _mm_add_ps(xy4, _mm_shuffle_ps::<0xb1>(xy4, xy4));
            xy = _mm_cvtss_f32(xy4);
            yy4 = _mm_add_ps(yy4, _mm_shuffle_ps::<0x4e>(yy4, yy4));
            yy4 = _mm_add_ps(yy4, _mm_shuffle_ps::<0xb1>(yy4, yy4));
            yy = _mm_cvtss_f32(yy4);
        }
        *xp.add(nn) = -100.0;
        *xp.add(nn + 1) = -100.0;
        *xp.add(nn + 2) = -100.0;
        *yp.add(nn) = 100.0;
        *yp.add(nn + 1) = 100.0;
        *yp.add(nn + 2) = 100.0;
        celt_sig_assert!(pulses_left >= 0);

        /* This should never happen, but just in case it does (e.g. on silence)
           we fill the first bin with pulses. */
        if pulses_left > n + 3 {
            let tmp = pulses_left as f32;
            yy = MAC16_16(yy, tmp, tmp);
            yy = MAC16_16(yy, tmp, *yp);
            *ip += pulses_left;
            pulses_left = 0;
        }

        let mut i = 0i32;
        while i < pulses_left {
            /* The squared magnitude term gets added anyway, so we might as well
               add it outside the loop */
            yy = ADD16(yy, 1.0);
            let xy4 = _mm_set1_ps(xy);
            let yy4 = _mm_set1_ps(yy);
            let mut max = _mm_setzero_ps();
            let mut pos = _mm_setzero_si128();
            let mut count = _mm_set_epi32(3, 2, 1, 0);
            let mut j = 0usize;
            while j < nn {
                let x4 = _mm_add_ps(_mm_loadu_ps(xp.add(j)), xy4);
                let y4 = _mm_add_ps(_mm_loadu_ps(yp.add(j)), yy4);
                let y4 = _mm_rsqrt_ps(y4);
                let r4 = _mm_mul_ps(x4, y4);
                /* Update the index of the max. */
                pos = _mm_max_epi16(
                    pos,
                    _mm_and_si128(count, _mm_castps_si128(_mm_cmpgt_ps(r4, max))),
                );
                /* Update the max. */
                max = _mm_max_ps(max, r4);
                /* Update the indices (+4) */
                count = _mm_add_epi32(count, fours);
                j += 4;
            }
            /* Horizontal max */
            let mut max2 = _mm_max_ps(max, _mm_shuffle_ps::<0x4e>(max, max));
            max2 = _mm_max_ps(max2, _mm_shuffle_ps::<0xb1>(max2, max2));
            /* Now that max2 contains the max at all positions, look at which
               value(s) of the partial max is equal to the global max. */
            pos = _mm_and_si128(pos, _mm_castps_si128(_mm_cmpeq_ps(max, max2)));
            pos = _mm_max_epi16(pos, _mm_unpackhi_epi64(pos, pos));
            pos = _mm_max_epi16(pos, _mm_shufflelo_epi16::<0x4e>(pos));
            let best_id = _mm_cvtsi128_si32(pos) as usize;

            /* Updating the sums of the new pulse(s) */
            xy = ADD32(xy, *xp.add(best_id));
            /* We're multiplying y[j] by two so we don't have to do it here */
            yy = ADD16(yy, *yp.add(best_id));

            /* Only now that we've made the final choice, update y/iy */
            /* Multiplying y[j] by 2 so we don't have to do it everywhere else */
            *yp.add(best_id) += 2.0;
            *ip.add(best_id) += 1;
            i += 1;
        }

        /* Put the original sign back */
        let mut j = 0usize;
        while j < nn {
            let y4 = _mm_loadu_si128(ip.add(j) as *const __m128i);
            let s4 = _mm_loadu_si128(sp.add(j) as *const __m128i);
            let r = _mm_xor_si128(_mm_add_epi32(y4, s4), s4);
            _mm_storeu_si128(ip.add(j) as *mut __m128i, r);
            j += 4;
        }
        yy
    }
}

/// `celt/vq.c:205` `op_pvq_search_c` — the plain-C reference (not reachable in
/// the reference configuration; kept for the probe).  Unlike the SSE2 version
/// it *does* modify `x` in place (sign-stripped magnitudes) and uses `1.f/sum`
/// instead of `_mm_rcp_ps`.  Needs `x.len() >= N`, `iy.len() >= N`.
pub fn op_pvq_search_c(x: &mut [f32], iy: &mut [i32], k: i32, n: i32) -> f32 {
    debug_assert!(n > 0);
    debug_assert!(k > 0);
    let nn = n as usize;
    debug_assert!(x.len() >= nn);
    debug_assert!(iy.len() >= nn);
    // `VARDECL(celt_norm, y); VARDECL(int, signx); ALLOC(..., N, ...)`
    // (`celt/vq.c`) — stack storage for the codec's `N <= MAX_PVQ_N` bands,
    // with a heap fallback for the `kernels` probe's unreachable sizes.
    let mut y_heap: Vec<f32> = Vec::new();
    let mut signx_heap: Vec<i32> = Vec::new();
    let mut y_storage = [0.0f32; MAX_PVQ_N];
    let mut signx_storage = [0i32; MAX_PVQ_N];
    let y: &mut [f32] = if nn <= MAX_PVQ_N {
        &mut y_storage[..nn]
    } else {
        y_heap.resize(nn, 0.0);
        y_heap.as_mut_slice()
    };
    let signx: &mut [i32] = if nn <= MAX_PVQ_N {
        &mut signx_storage[..nn]
    } else {
        signx_heap.resize(nn, 0);
        signx_heap.as_mut_slice()
    };

    // Get rid of the sign
    let mut sum = 0.0f32;
    let mut j = 0usize;
    while j < nn {
        signx[j] = if x[j] < 0.0 { 1 } else { 0 };
        // OPT: Make sure the compiler doesn't use a branch on ABS16().
        x[j] = ABS16(x[j]);
        iy[j] = 0;
        y[j] = 0.0;
        j += 1;
    }

    let mut xy = 0.0f32;
    let mut yy = 0.0f32;
    let mut pulses_left = k;

    // Do a pre-search by projecting on the pyramid
    if k > (n >> 1) {
        let mut j = 0usize;
        while j < nn {
            sum += x[j];
            j += 1;
        }
        // Prevents infinities and NaNs from causing too many pulses to be
        // allocated.  64 is an approximation of infinity here.
        if !(sum > EPSILON && sum < 64.0) {
            x[0] = QCONST16(1.0, 14);
            let mut j = 1usize;
            while j < nn {
                x[j] = 0.0;
                j += 1;
            }
            sum = QCONST16(1.0, 14);
        }
        // Using K+e with e < 1 guarantees we cannot get more than K pulses.
        let rcp = MULT16_32_Q16(k as f32 + 0.8f32, celt_rcp(sum));
        let mut j = 0usize;
        while j < nn {
            // It's really important to round *towards zero* here
            iy[j] = (rcp * x[j]).floor() as i32;
            y[j] = iy[j] as f32;
            yy = MAC16_16(yy, y[j], y[j]);
            xy = MAC16_16(xy, x[j], y[j]);
            y[j] *= 2.0;
            pulses_left -= iy[j];
            j += 1;
        }
    }
    celt_sig_assert!(pulses_left >= 0);

    if pulses_left > n + 3 {
        let tmp = pulses_left as f32;
        yy = MAC16_16(yy, tmp, tmp);
        yy = MAC16_16(yy, tmp, y[0]);
        iy[0] += pulses_left;
        pulses_left = 0;
    }

    let mut i = 0i32;
    while i < pulses_left {
        // The squared magnitude term gets added anyway, so we might as well
        // add it outside the loop
        yy = ADD16(yy, 1.0);

        // Calculations for position 0 are out of the loop
        let mut rxy = ADD32(xy, x[0]);
        let mut ryy = ADD16(yy, y[0]);
        rxy = MULT16_16_Q15(rxy, rxy);
        let mut best_den = ryy;
        let mut best_num = rxy;
        let mut best_id = 0usize;
        let mut j = 1usize;
        while j < nn {
            rxy = ADD32(xy, x[j]);
            ryy = ADD16(yy, y[j]);
            rxy = MULT16_16_Q15(rxy, rxy);
            if MULT16_16(best_den, rxy) > MULT16_16(ryy, best_num) {
                best_den = ryy;
                best_num = rxy;
                best_id = j;
            }
            j += 1;
        }

        // Updating the sums of the new pulse(s)
        xy = ADD32(xy, x[best_id]);
        yy = ADD16(yy, y[best_id]);

        // Only now that we've made the final choice, update y/iy
        y[best_id] += 2.0;
        iy[best_id] += 1;
        i += 1;
    }

    // Put the original sign back
    let mut j = 0usize;
    while j < nn {
        iy[j] = (iy[j] ^ -signx[j]).wrapping_add(signx[j]);
        j += 1;
    }
    yy
}

// ---------------------------------------------------------------------------
// [fast] PVQ search specialisations
//
// The dispatch shape follows `restsend/opus-rs` `src/pvq.rs:100-147`
// (`pvq_search`): `K == 1` is an argmax, `N == 2` uses the exact reciprocal,
// `N == 4` has a dedicated four-lane body, and everything else runs through a
// specialised kernel.  `N == 2` (closed form, exact reciprocal) and `K == 1`
// (argmax, see its doc comment for the tie-break difference) move the
// bitstream; the `N == 4` and general bodies keep the reference score
// function, the `_mm_rcp_ps` / `_mm_rsqrt_ps` approximations and the
// tie-break, so they are bit-identical to [`op_pvq_search_sse2`] (verified by
// a 2912-case equivalence sweep against the reference body).
//
// The general body's win is *not* a wider vector: the reference's
// `VARDECL(celt_norm, X/y/signy)` are three `[0.0; MAX_PVQ_N + 3]` arrays, and
// zero-filling 3 x 4.3 KiB of scratch on every call costs more than a small
// band's search (the C uses an uninitialised `alloca`).  [`op_pvq_search_fast_body`]
// keeps the four-lane reference arithmetic but writes only the lanes it reads.
// An eight-lane AVX2 scan was implemented, measured and reverted: the per-pulse
// cost is dominated by the serial `xy`/`yy` update, the horizontal argmax and
// the scalar `y[best] += 2` store, so halving the loop count did not pay (see
// `docs/reports/pvq-specialisations.md`).
// ---------------------------------------------------------------------------

/// `[fast]` `K == 1`: a single unit pulse on the largest magnitude.
///
/// For `K == 1` the reference body's whole greedy loop is a single pass with
/// `xy == 0` and `yy == 1`, so every lane scores exactly `|x|` and the pulse
/// goes to a maximum-magnitude coefficient.  Its tie-break, however, is an
/// artefact of the `_mm_max_epi16` bookkeeping: `pos` is only raised on a
/// *strict* improvement over the running per-lane maximum, so among equal
/// magnitudes the winner is the largest index of the earliest four-lane group
/// that reached the maximum, not the largest index overall (measured:
/// `|x|` all equal, `n = 39` -> reference index 3, this body index 38).
///
/// This body is the plain argmax (largest index among equal magnitudes,
/// index 0 when all are zero), which is the same *choice of coefficient* the
/// reference is reaching for and is what `restsend/opus-rs` does
/// (`pvq_search`, `k == 1`).  It is therefore not bit-identical on ties; the
/// measured cost is a different (equally scoring) pulse position, zero size
/// drift and a `0.00 dB` signal-SNR delta (`docs/reports/pvq-specialisations.md`).
///
/// # Slice lengths
/// `x.len() >= n`, `iy.len() >= n + 3` (same contract as
/// [`op_pvq_search_sse2`]).
///
/// Returns `yy = <iy,iy> = 1`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
fn op_pvq_search_k1(x: &[f32], iy: &mut [i32], n: usize) -> f32 {
    debug_assert!(n > 0 && x.len() >= n && iy.len() >= n + 3);
    let mut best = 0usize;
    let mut best_abs = 0.0f32;
    let mut j = n;
    while j > 0 {
        j -= 1;
        let a = x[j].abs();
        if a > best_abs {
            best_abs = a;
            best = j;
        }
    }
    iy[..n].fill(0);
    iy[best] = if x[best] < 0.0 { -1 } else { 1 };
    1.0
}

/// `[fast]` `N == 2`: closed-form split with an exact `1.0 / sum` reciprocal
/// instead of the reference's `_mm_rcp_ps` approximation.
///
/// With two dimensions the PVQ optimum is `y ∝ |X|`, so rounding `y0` to the
/// nearest integer and giving `K - y0` to `y1` is the best integer split; the
/// reference instead projects, truncates and then runs the greedy loop.  The
/// pulse vector therefore differs, but both are valid codewords
/// (`|y|_1 == K`).
///
/// Returns `yy = <iy,iy>`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
fn op_pvq_search_n2(x: &[f32], iy: &mut [i32], k: i32) -> f32 {
    debug_assert!(k > 0 && x.len() >= 2 && iy.len() >= 5);
    let a0 = x[0].abs();
    let a1 = x[1].abs();
    let sum = a0 + a1;
    if !(sum > EPSILON) {
        /* All-zero (or NaN) input: the reference's "pulse at 0" rule. */
        iy[0] = k;
        iy[1] = 0;
        return (k as f32) * (k as f32);
    }
    let rcp = 1.0f32 / sum;
    let kf = k as f32;
    let y0 = ((kf * a0 * rcp + 0.5).floor() as i32).clamp(0, k);
    let y1 = k - y0;
    iy[0] = if x[0] < 0.0 { -y0 } else { y0 };
    iy[1] = if x[1] < 0.0 { -y1 } else { y1 };
    (y0 * y0 + y1 * y1) as f32
}

/// `[fast]` `N == 4`: the whole search on one `__m128`, with the four-lane
/// argmax recovered by `_mm_movemask_ps` instead of the reference's
/// `_mm_max_epi16` position vector.
///
/// The score function, the pre-search (including `_mm_rcp_ps`), the reduction
/// order and the "largest index among the lanes equal to the maximum"
/// tie-break are the reference's, so the result is bit-identical to
/// [`op_pvq_search_sse2`]; the win is dropping the `N + 3` scratch buffers and
/// the per-pulse index bookkeeping.
///
/// # Slice lengths
/// `x.len() >= 4`, `iy.len() >= 7` (the four-lane `iy` store writes `iy[0..4]`).
///
/// Returns `yy = <iy,iy>`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn op_pvq_search_n4_hw(x: &[f32], iy: &mut [i32], k: i32) -> f32 {
    use core::arch::x86_64::*;
    /* SAFETY: SSE2 is the x86-64 baseline; the caller guarantees the slice
       lengths (`x.len() >= 4`, `iy.len() >= 7`). */
    unsafe {
        let signmask = _mm_set_ps1(-0.0);
        let zero = _mm_setzero_ps();
        let vx = _mm_loadu_ps(x.as_ptr());
        let sign4 = _mm_cmplt_ps(vx, zero);
        let mut xv = _mm_andnot_ps(signmask, vx);
        let mut xabs = [0.0f32; 4];
        let mut y2f = [0.0f32; 4];
        let mut iys = [0i32; 4];
        let mut xy = 0.0f32;
        let mut yy = 0.0f32;
        let mut pulses_left = k;

        if k > 2 {
            /* `sums`, with the reference's two-shuffle reduction. */
            let mut sums = _mm_add_ps(xv, _mm_shuffle_ps::<0x4e>(xv, xv));
            sums = _mm_add_ps(sums, _mm_shuffle_ps::<0xb1>(sums, sums));
            let sum = _mm_cvtss_f32(sums);
            if !(sum > EPSILON && sum < 64.0) {
                xv = _mm_set_ps(0.0, 0.0, 0.0, QCONST16(1.0, 14));
                sums = _mm_set1_ps(1.0);
            }
            let rcp4 = _mm_mul_ps(_mm_set1_ps((k as f64 + 0.8) as f32), _mm_rcp_ps(sums));
            let rx4 = _mm_mul_ps(xv, rcp4);
            let iy4 = _mm_cvttps_epi32(rx4);
            let y4 = _mm_cvtepi32_ps(iy4);
            _mm_storeu_si128(iys.as_mut_ptr() as *mut __m128i, iy4);
            let xy4 = _mm_mul_ps(xv, y4);
            let yy4 = _mm_mul_ps(y4, y4);
            let mut t = _mm_add_ps(xy4, _mm_shuffle_ps::<0x4e>(xy4, xy4));
            t = _mm_add_ps(t, _mm_shuffle_ps::<0xb1>(t, t));
            xy = _mm_cvtss_f32(t);
            let mut t = _mm_add_ps(yy4, _mm_shuffle_ps::<0x4e>(yy4, yy4));
            t = _mm_add_ps(t, _mm_shuffle_ps::<0xb1>(t, t));
            yy = _mm_cvtss_f32(t);
            let mut t = _mm_add_epi32(iy4, _mm_shuffle_epi32::<0x4e>(iy4));
            t = _mm_add_epi32(t, _mm_shuffle_epi32::<0xb1>(t));
            pulses_left -= _mm_cvtsi128_si32(t);
            _mm_storeu_ps(y2f.as_mut_ptr(), _mm_add_ps(y4, y4));
        }
        _mm_storeu_ps(xabs.as_mut_ptr(), xv);
        celt_sig_assert!(pulses_left >= 0);

        /* This should never happen, but just in case it does (e.g. on
           silence) we fill the first bin with pulses. */
        if pulses_left > 7 {
            let tmp = pulses_left as f32;
            yy = MAC16_16(yy, tmp, tmp);
            yy = MAC16_16(yy, tmp, y2f[0]);
            iys[0] += pulses_left;
            pulses_left = 0;
        }

        let xq = _mm_loadu_ps(xabs.as_ptr());
        let mut i = 0i32;
        while i < pulses_left {
            /* The squared magnitude term gets added anyway, so we might as
               well add it outside the loop. */
            yy = ADD16(yy, 1.0);
            let rxy = _mm_add_ps(xq, _mm_set1_ps(xy));
            let ryy = _mm_add_ps(_mm_loadu_ps(y2f.as_ptr()), _mm_set1_ps(yy));
            let score = _mm_mul_ps(rxy, _mm_rsqrt_ps(ryy));
            /* Horizontal max; then every lane equal to it is a candidate and
               the largest index wins, exactly like the reference. */
            let mut m = _mm_max_ps(score, _mm_shuffle_ps::<0x4e>(score, score));
            m = _mm_max_ps(m, _mm_shuffle_ps::<0xb1>(m, m));
            let mask = _mm_movemask_ps(_mm_cmpeq_ps(score, m)) as u32;
            let best = (31 - mask.leading_zeros()) as usize;
            xy = ADD32(xy, xabs[best]);
            yy = ADD16(yy, y2f[best]);
            y2f[best] += 2.0;
            iys[best] += 1;
            i += 1;
        }

        /* Put the original sign back: `iy = (iy ^ sign) + sign` with
           `sign = -1` for a negative input. */
        /* `s4` is the `_mm_cmplt_ps` mask (-1 for a negative input), so the
           reference's `(y ^ s) + s` on the integer `signx` becomes
           `(y + s) ^ s` here. */
        let signs = _mm_movemask_ps(sign4) as u32;
        let mut j = 0usize;
        while j < 4 {
            let s = -(((signs >> j) & 1) as i32);
            iy[j] = (iys[j].wrapping_add(s)) ^ s;
            j += 1;
        }
        yy
    }
}

/// `[fast]` general-body specialisation: the reference
/// [`op_pvq_search_sse2_hw`] arithmetic on scratch that is *written before it
/// is read* instead of being zero-filled.
///
/// The reference wrapper allocates three `[0.0f32; MAX_PVQ_N + 3]` arrays
/// (4.3 KiB) per call; for a small band the zero-fill is a large share of the
/// call.  The C `ALLOC` is an `alloca`, which does not initialise, so this body
/// reproduces that layout: `X[0..N]` is copied in, `X[N..N+3]` is zeroed (the
/// reference's `X[N] = X[N+1] = X[N+2] = 0`), and `y`/`signy` are left
/// uninitialised because the first four-lane pass writes every lane the kernel
/// later reads (`y[N..N+3] = 100` is written by the reference body itself).
///
/// The pulse vector and `yy` are bit-identical to [`op_pvq_search_sse2`].
///
/// # Slice lengths
/// `x.len() >= n`, `iy.len() >= n + 3`.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
fn op_pvq_search_fast_body(x: &[f32], iy: &mut [i32], k: i32, n: usize) -> f32 {
    use core::mem::MaybeUninit;
    let nn = n;
    let mut xx_storage = [MaybeUninit::<f32>::uninit(); MAX_PVQ_N + 3];
    let mut y_storage = [MaybeUninit::<f32>::uninit(); MAX_PVQ_N + 3];
    let mut signy_storage = [MaybeUninit::<f32>::uninit(); MAX_PVQ_N + 3];
    let mut xx_heap: Vec<f32> = Vec::new();
    let mut y_heap: Vec<f32> = Vec::new();
    let mut signy_heap: Vec<f32> = Vec::new();
    let (xp, yp, sp): (*mut f32, *mut f32, *mut f32) = if nn + 3 <= MAX_PVQ_N + 3 {
        (
            xx_storage.as_mut_ptr().cast::<f32>(),
            y_storage.as_mut_ptr().cast::<f32>(),
            signy_storage.as_mut_ptr().cast::<f32>(),
        )
    } else {
        /* The `kernels` probe's unreachable sizes. */
        xx_heap.resize(nn + 3, 0.0);
        y_heap.resize(nn + 3, 0.0);
        signy_heap.resize(nn + 3, 0.0);
        (
            xx_heap.as_mut_ptr(),
            y_heap.as_mut_ptr(),
            signy_heap.as_mut_ptr(),
        )
    };
    /* SAFETY: each pointer has `nn + 3` writable floats.  `x.len() >= nn`
       (asserted by `op_pvq_search_sse2`), so the copy stays in bounds; the
       three tail lanes are the reference's `X[N..N+3] = 0`; `y` and `signy`
       are written by the kernel's first pass before any read. */
    unsafe {
        core::ptr::copy_nonoverlapping(x.as_ptr(), xp, nn);
        core::ptr::write_bytes(xp.add(nn), 0, 3);
        op_pvq_search_sse2_hw(xp, yp, sp, iy, k, nn as i32)
    }
}

/// Dispatcher for the `[fast]` PVQ specialisations.  Returns `None` when the
/// reference [`op_pvq_search_sse2`] body should run instead.
#[cfg(all(feature = "fast", target_arch = "x86_64"))]
#[inline]
fn op_pvq_search_specialised(x: &[f32], iy: &mut [i32], k: i32, n: usize) -> Option<f32> {
    /* `K == 1` first: it also covers `N == 2`, and the argmax is the whole
       search. */
    if k == 1 {
        return Some(op_pvq_search_k1(x, iy, n));
    }
    if n == 2 {
        return Some(op_pvq_search_n2(x, iy, k));
    }
    if n == 4 {
        /* SAFETY: the caller checks `iy.len() >= n + 3`. */
        return Some(unsafe { op_pvq_search_n4_hw(x, iy, k) });
    }
    Some(op_pvq_search_fast_body(x, iy, k, n))
}

/// `op_pvq_search` as used by the port.  In the reference configuration
/// `OPUS_X86_PRESUME_SSE2` makes `arch` a no-op and the SSE2 kernel is used at
/// every value, so `arch` is accepted for signature compatibility and ignored.
/// See [`op_pvq_search_sse2`].
pub fn op_pvq_search(x: &mut [f32], iy: &mut [i32], k: i32, n: i32, arch: i32) -> f32 {
    op_pvq_search_sse2(x, iy, k, n, arch)
}

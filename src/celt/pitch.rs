/* Copyright (c) 2007-2008 CSIRO
   Copyright (c) 2007-2009 Xiph.Org Foundation
   Written by Jean-Marc Valin */
/* @file pitch.c
   @brief Pitch analysis
 */

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

//! Transliteration of `celt/pitch.h` and `celt/pitch.c` (libopus 1.6.1) into
//! safe Rust.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! ## Configuration (pinned reference build: float, x86 RTCD, arch 4)
//!
//! * `FIXED_POINT` is undefined, so every `#ifdef FIXED_POINT` branch is
//!   dropped: `find_best_pitch` loses its `yshift`/`maxcorr` parameters and
//!   its `xshift` local (the `SHR32`/`VSHR32` macros that mention them are the
//!   identity), `compute_pitch_gain` is `xy/celt_sqrt(1+xx*yy)`, and
//!   `pitch_downsample` takes the float mixing branch.
//! * `xcorr_kernel` and `celt_inner_prod` are **not** runtime-dispatched:
//!   with `OPUS_X86_PRESUME_SSE` defined the macros in `celt/x86/pitch_sse.h`
//!   expand directly to `xcorr_kernel_sse` / `celt_inner_prod_sse`, which is
//!   exactly what [`crate::celt::kernels`] provides.  `celt_pitch_xcorr` is
//!   the one real RTCD table (`PITCH_XCORR_IMPL`); `arch == 4` selects
//!   `celt_pitch_xcorr_avx2`, i.e.
//!   [`crate::celt::kernels::celt_pitch_xcorr`].
//! * `OVERRIDE_PITCH_XCORR` *is* defined (`celt/x86/pitch_sse.h:198`), but
//!   `celt_pitch_xcorr_c` is nevertheless compiled: the only
//!   `#if !defined(OVERRIDE_PITCH_XCORR)` in `celt/pitch.c:240` guards the
//!   `(void)arch;` inside the `#if 0` branch, and the `PITCH_XCORR_IMPL`
//!   table's `arch 0..3` entries reference the C function.
//!   `nm reference/lib-ref/libopus.a` shows `T celt_pitch_xcorr_c`, which is
//!   [`crate::celt::kernels::celt_pitch_xcorr_c`].
//! * The `arch` argument is threaded through the C API.  [`crate::celt::kernels`]
//!   has no `arch` parameter (the presumed-SSE kernels ignore it and the AVX2
//!   entry is selected by the caller), so the call sites pass the value
//!   through to the functions that still keep it (`_celt_autocorr`) and drop
//!   it where the C macro did (`(void)arch`).  The parameter itself is kept on
//!   every public function so the C parameter list is preserved.
//!
//! ## Index conventions
//!
//! `remove_doubling` performs `x += maxperiod` on its pointer, so the C reads
//! `x[-i]`, `x[N-i]`, `x-T0`, ... relative to that shifted pointer.  A Rust
//! slice cannot be indexed negatively, so the `x` slice that [`remove_doubling`]
//! expects is the one the C *caller* passes (BEFORE the `x += maxperiod`),
//! and every access is offset by `maxperiod/2` internally.
//!
//! `pitch_downsample` takes `celt_sig *x[]` (one pointer per channel) and
//! becomes `&[&[celt_sig]]`; `x_lp` is written in place.

use super::arch::*;
use super::celt_lpc::{_celt_autocorr, _celt_lpc};
use super::entcode::celt_udiv;
use super::kernels;
use super::mathops::{celt_sqrt, frac_div32};
use crate::types::*;

/// `celt/pitch.c:45` `find_best_pitch` (static in C; `pub` here so the probe
/// can exercise it directly — the reference source is compiled into the probe
/// for that purpose).
///
/// `xcorr.len() >= max_pitch` and `y.len() >= max_pitch + len` (the C reads
/// `y[i+len]` for `i < max_pitch`).
pub fn find_best_pitch(
    xcorr: &[opus_val32],
    y: &[opus_val16],
    len: i32,
    max_pitch: i32,
    best_pitch: &mut [i32; 2],
) {
    let mut syy: opus_val32 = 1.;
    let mut best_num: [opus_val16; 2] = [-1., -1.];
    let mut best_den: [opus_val32; 2] = [0., 0.];
    // In the float build `yshift`/`xshift` do not exist: SHR32/VSHR32 are the
    // identity, so the shift argument is written as 0.
    best_pitch[0] = 0;
    best_pitch[1] = 1;
    for j in 0..len as usize {
        syy = ADD32(syy, SHR32(MULT16_16(y[j], y[j]), 0));
    }
    for i in 0..max_pitch {
        let ii = i as usize;
        if xcorr[ii] > 0. {
            let mut xcorr16: opus_val16 = EXTRACT16(VSHR32(xcorr[ii], 0));
            /* Considering the range of xcorr16, this should avoid both underflows
               and overflows (inf) when squaring xcorr16 */
            xcorr16 *= 1e-12f32;
            let num: opus_val16 = MULT16_16_Q15(xcorr16, xcorr16);
            if MULT16_32_Q15(num, best_den[1]) > MULT16_32_Q15(best_num[1], syy) {
                if MULT16_32_Q15(num, best_den[0]) > MULT16_32_Q15(best_num[0], syy) {
                    best_num[1] = best_num[0];
                    best_den[1] = best_den[0];
                    best_pitch[1] = best_pitch[0];
                    best_num[0] = num;
                    best_den[0] = syy;
                    best_pitch[0] = i;
                } else {
                    best_num[1] = num;
                    best_den[1] = syy;
                    best_pitch[1] = i;
                }
            }
        }
        syy += SHR32(MULT16_16(y[ii + len as usize], y[ii + len as usize]), 0)
            - SHR32(MULT16_16(y[ii], y[ii]), 0);
        syy = MAX32(1., syy);
    }
}

/// `celt/pitch.c:105` `celt_fir5` (static in C).
///
/// Five-tap FIR applied in place; `num` holds the five taps and `x` the `N`
/// samples.
pub fn celt_fir5(x: &mut [opus_val16], num: &[opus_val16], n: i32) {
    let num0 = num[0];
    let num1 = num[1];
    let num2 = num[2];
    let num3 = num[3];
    let num4 = num[4];
    let mut mem0: opus_val32 = 0.;
    let mut mem1: opus_val32 = 0.;
    let mut mem2: opus_val32 = 0.;
    let mut mem3: opus_val32 = 0.;
    let mut mem4: opus_val32 = 0.;
    for i in 0..n as usize {
        let mut sum: opus_val32 = SHL32(EXTEND32(x[i]), SIG_SHIFT);
        sum = MAC16_16(sum, num0, mem0);
        sum = MAC16_16(sum, num1, mem1);
        sum = MAC16_16(sum, num2, mem2);
        sum = MAC16_16(sum, num3, mem3);
        sum = MAC16_16(sum, num4, mem4);
        mem4 = mem3;
        mem3 = mem2;
        mem2 = mem1;
        mem1 = mem0;
        mem0 = x[i];
        x[i] = ROUND16(sum, SIG_SHIFT);
    }
}

/// `celt/pitch.c:140` `pitch_downsample`.
///
/// `x[c]` is channel `c`; it must provide at least
/// `factor*(len-1) + factor/2 + 1` samples.  `x_lp` is written for
/// `0..len`.  `arch` is forwarded to [`_celt_autocorr`].
pub fn pitch_downsample(
    x: &[&[celt_sig]],
    x_lp: &mut [opus_val16],
    len: i32,
    c: i32,
    factor: i32,
    arch: i32,
) {
    let mut ac = [0 as opus_val32; 5];
    let mut tmp: opus_val16 = Q15ONE;
    let mut lpc = [0 as opus_val16; 4];
    let mut lpc2 = [0 as opus_val16; 5];
    let c1 = QCONST16(0.8f32, 15);
    let offset = factor / 2;
    debug_assert!(len > 0);
    debug_assert!(c == 1 || c == 2);
    for i in 1..len as usize {
        let j = factor * i as i32;
        x_lp[i] = 0.25f32 * x[0][(j - offset) as usize]
            + 0.25f32 * x[0][(j + offset) as usize]
            + 0.5f32 * x[0][j as usize];
    }
    x_lp[0] = 0.25f32 * x[0][offset as usize] + 0.5f32 * x[0][0];
    if c == 2 {
        for i in 1..len as usize {
            let j = factor * i as i32;
            x_lp[i] += 0.25f32 * x[1][(j - offset) as usize]
                + 0.25f32 * x[1][(j + offset) as usize]
                + 0.5f32 * x[1][j as usize];
        }
        x_lp[0] += 0.25f32 * x[1][offset as usize] + 0.5f32 * x[1][0];
    }
    _celt_autocorr(x_lp, &mut ac, None, 0, 4, len, arch);

    /* Noise floor -40 dB */
    ac[0] *= 1.0001f32;
    /* Lag windowing */
    for i in 1..=4 {
        /*ac[i] *= exp(-.5*(2*M_PI*.002*i)*(2*M_PI*.002*i));*/
        ac[i] -= ac[i] * (0.008f32 * i as f32) * (0.008f32 * i as f32);
    }

    _celt_lpc(&mut lpc, &ac, 4);
    for i in 0..4 {
        tmp = MULT16_16_Q15(QCONST16(0.9f32, 15), tmp);
        lpc[i] = MULT16_16_Q15(lpc[i], tmp);
    }
    /* Add a zero */
    lpc2[0] = lpc[0] + QCONST16(0.8f32, SIG_SHIFT);
    lpc2[1] = lpc[1] + MULT16_16_Q15(c1, lpc[0]);
    lpc2[2] = lpc[2] + MULT16_16_Q15(c1, lpc[1]);
    lpc2[3] = lpc[3] + MULT16_16_Q15(c1, lpc[2]);
    lpc2[4] = MULT16_16_Q15(c1, lpc[3]);
    celt_fir5(x_lp, &lpc2, len);
}

/// `celt/pitch.c:230` `celt_pitch_xcorr_c` — the plain-C entry point selected
/// by `PITCH_XCORR_IMPL[arch]` for `arch <= 3`.  (It is compiled even though
/// `OVERRIDE_PITCH_XCORR` is defined; see the module docs.)  In this
/// configuration its `xcorr_kernel`/`celt_inner_prod` macros still resolve to
/// the SSE kernels, so only the 4-way split differs from the AVX2 entry
/// point.
pub fn celt_pitch_xcorr_c(
    x: &[opus_val16],
    y: &[opus_val16],
    xcorr: &mut [opus_val32],
    len: i32,
    max_pitch: i32,
    _arch: i32,
) {
    kernels::celt_pitch_xcorr_c(x, y, xcorr, len, max_pitch);
}

/// `celt/pitch.c:307` `pitch_search`.
///
/// `x_lp` and `y` point at the buffers the C caller passes; `y` must provide
/// `len + max_pitch` samples and `x_lp` at least `len/2 + 1` (the fine search
/// reads `x_lp[0..len/2]`).  `*pitch` receives the chosen lag.
pub fn pitch_search(
    x_lp: &[opus_val16],
    y: &[opus_val16],
    len: i32,
    max_pitch: i32,
    pitch: &mut i32,
    arch: i32,
) {
    let mut best_pitch: [i32; 2] = [0, 0];
    /* arch is only consumed by the runtime-dispatched celt_pitch_xcorr below;
       the SSE kernels themselves have no arch argument. */
    let _ = arch;
    celt_assert!(len > 0);
    celt_assert!(max_pitch > 0);
    let lag = len + max_pitch;

    let mut x_lp4 = vec![0 as opus_val16; (len >> 2) as usize];
    let mut y_lp4 = vec![0 as opus_val16; (lag >> 2) as usize];
    let mut xcorr = vec![0 as opus_val32; (max_pitch >> 1) as usize];

    /* Downsample by 2 again */
    for j in 0..(len >> 2) as usize {
        x_lp4[j] = x_lp[2 * j];
    }
    for j in 0..(lag >> 2) as usize {
        y_lp4[j] = y[2 * j];
    }

    /* Coarse search with 4x decimation */
    // celt_pitch_xcorr(..., arch) -> PITCH_XCORR_IMPL[arch & OPUS_ARCHMASK];
    // arch == 4 selects celt_pitch_xcorr_avx2.
    kernels::celt_pitch_xcorr(&x_lp4, &y_lp4, &mut xcorr, len >> 2, max_pitch >> 2);

    find_best_pitch(&xcorr, &y_lp4, len >> 2, max_pitch >> 2, &mut best_pitch);

    /* Finer search with 2x decimation */
    for i in 0..(max_pitch >> 1) as usize {
        xcorr[i] = 0.;
        if (i as i32 - 2 * best_pitch[0]).abs() > 2 && (i as i32 - 2 * best_pitch[1]).abs() > 2 {
            continue;
        }
        let sum = kernels::celt_inner_prod(x_lp, &y[i..], len >> 1);
        xcorr[i] = MAX32(-1., sum);
    }
    find_best_pitch(&xcorr, y, len >> 1, max_pitch >> 1, &mut best_pitch);

    /* Refine by pseudo-interpolation */
    let offset;
    if best_pitch[0] > 0 && best_pitch[0] < (max_pitch >> 1) - 1 {
        let a = xcorr[(best_pitch[0] - 1) as usize];
        let b = xcorr[best_pitch[0] as usize];
        let c = xcorr[(best_pitch[0] + 1) as usize];
        if (c - a) > MULT16_32_Q15(QCONST16(0.7f32, 15), b - a) {
            offset = 1;
        } else if (a - c) > MULT16_32_Q15(QCONST16(0.7f32, 15), b - c) {
            offset = -1;
        } else {
            offset = 0;
        }
    } else {
        offset = 0;
    }
    *pitch = 2 * best_pitch[0] - offset;
}

/// `celt/pitch.c:447` `compute_pitch_gain` (static in C, float branch).
fn compute_pitch_gain(xy: opus_val32, xx: opus_val32, yy: opus_val32) -> opus_val16 {
    xy / celt_sqrt(1. + xx * yy)
}

/// `celt/pitch.c:453` `second_check` (static const in C).
const second_check: [i32; 16] = [0, 0, 3, 2, 3, 2, 5, 2, 3, 2, 3, 2, 5, 2, 3, 2];

/// `celt/pitch.c:454` `remove_doubling`.
///
/// The C does `x += maxperiod` after halving every period argument; the `x`
/// slice passed here is the C caller's pointer *before* that shift (see the
/// module docs), and all accesses use `maxperiod/2` as their base offset.
pub fn remove_doubling(
    x: &[opus_val16],
    maxperiod: i32,
    minperiod: i32,
    n: i32,
    t0_: &mut i32,
    prev_period: i32,
    prev_gain: opus_val16,
    arch: i32,
) -> opus_val16 {
    let _ = arch;
    let minperiod0 = minperiod;
    let maxperiod = maxperiod / 2;
    let minperiod = minperiod / 2;
    *t0_ /= 2;
    let prev_period = prev_period / 2;
    let n = n / 2;
    /* x += maxperiod (the pointer shift of the C source). */
    let xb = maxperiod as usize;
    if *t0_ >= maxperiod {
        *t0_ = maxperiod - 1;
    }

    let mut t = *t0_;
    let t0 = *t0_;
    let mut yy_lookup = vec![0 as opus_val32; (maxperiod + 1) as usize];
    let mut xx: opus_val32 = 0.;
    let mut xy: opus_val32 = 0.;
    kernels::dual_inner_prod(
        &x[xb..],
        &x[xb..],
        &x[xb - t0 as usize..],
        n,
        &mut xx,
        &mut xy,
    );
    yy_lookup[0] = xx;
    let mut yy = xx;
    for i in 1..=maxperiod as usize {
        let ii = i as i32;
        yy = yy + MULT16_16(x[xb - i], x[xb - i])
            - MULT16_16(x[(xb as i32 + n - ii) as usize], x[(xb as i32 + n - ii) as usize]);
        yy_lookup[i] = MAX32(0., yy);
    }
    yy = yy_lookup[t0 as usize];
    let mut best_xy = xy;
    let mut best_yy = yy;
    let mut g = compute_pitch_gain(xy, xx, yy);
    let g0 = g;
    /* Look for any pitch at T/k */
    for k in 2..=15i32 {
        let t1 = celt_udiv((2 * t0 + k) as opus_uint32, (2 * k) as opus_uint32) as i32;
        if t1 < minperiod {
            break;
        }
        /* Look for another strong correlation at T1b */
        let t1b = if k == 2 {
            if t1 + t0 > maxperiod {
                t0
            } else {
                t0 + t1
            }
        } else {
            celt_udiv(
                (2 * second_check[k as usize] * t0 + k) as opus_uint32,
                (2 * k) as opus_uint32,
            ) as i32
        };
        let mut xy: opus_val32 = 0.;
        let mut xy2: opus_val32 = 0.;
        kernels::dual_inner_prod(
            &x[xb..],
            &x[xb - t1 as usize..],
            &x[xb - t1b as usize..],
            n,
            &mut xy,
            &mut xy2,
        );
        xy = HALF32(xy + xy2);
        yy = HALF32(yy_lookup[t1 as usize] + yy_lookup[t1b as usize]);
        let g1 = compute_pitch_gain(xy, xx, yy);
        let cont: opus_val16 = if (t1 - prev_period).abs() <= 1 {
            prev_gain
        } else if (t1 - prev_period).abs() <= 2 && 5 * k * k < t0 {
            HALF16(prev_gain)
        } else {
            0.
        };
        let mut thresh = MAX16(
            QCONST16(0.3f32, 15),
            MULT16_16_Q15(QCONST16(0.7f32, 15), g0) - cont,
        );
        /* Bias against very high pitch (very short period) to avoid false-positives
           due to short-term correlation */
        if t1 < 3 * minperiod {
            thresh = MAX16(
                QCONST16(0.4f32, 15),
                MULT16_16_Q15(QCONST16(0.85f32, 15), g0) - cont,
            );
        } else if t1 < 2 * minperiod {
            thresh = MAX16(
                QCONST16(0.5f32, 15),
                MULT16_16_Q15(QCONST16(0.9f32, 15), g0) - cont,
            );
        }
        if g1 > thresh {
            best_xy = xy;
            best_yy = yy;
            t = t1;
            g = g1;
        }
    }
    best_xy = MAX32(0., best_xy);
    let mut pg: opus_val16;
    if best_yy <= best_xy {
        pg = Q15ONE;
    } else {
        pg = SHR32(frac_div32(best_xy, best_yy + 1.), 16);
    }

    let mut xcorr = [0 as opus_val32; 3];
    for k in 0..3i32 {
        let off = xb as i32 - (t + k - 1);
        debug_assert!(off >= 0, "remove_doubling: period exceeds the buffer");
        xcorr[k as usize] = kernels::celt_inner_prod(&x[xb..], &x[off as usize..], n);
    }
    let offset;
    if (xcorr[2] - xcorr[0]) > MULT16_32_Q15(QCONST16(0.7f32, 15), xcorr[1] - xcorr[0]) {
        offset = 1;
    } else if (xcorr[0] - xcorr[2]) > MULT16_32_Q15(QCONST16(0.7f32, 15), xcorr[1] - xcorr[2]) {
        offset = -1;
    } else {
        offset = 0;
    }
    if pg > g {
        pg = g;
    }
    *t0_ = 2 * t + offset;

    if *t0_ < minperiod0 {
        *t0_ = minperiod0;
    }
    pg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-tap `num = [1]` five-tap FIR feeds `x[i-1]` forward:
    /// `y[0] = x[0]`, `y[1] = x[1] + x[0]`, `y[2] = x[2] + x[1] = 0`.
    #[test]
    fn fir5_single_tap() {
        let mut x = [0.0f32; 8];
        x[0] = 1.0;
        let num = [1.0f32, 0.0, 0.0, 0.0, 0.0];
        celt_fir5(&mut x, &num, 8);
        assert_eq!(x[0], 1.0);
        assert_eq!(x[1], 1.0);
        assert_eq!(x[2], 0.0);
    }

    /// `find_best_pitch` picks the lag of a strong correlation and reports it
    /// in `best_pitch[0]`; the second candidate is `best_pitch[1]`.
    #[test]
    fn find_best_pitch_picks_peak() {
        let mut xcorr = [0.0f32; 8];
        xcorr[3] = 1000.0;
        let mut y = vec![0.0f32; 4 + 16];
        for i in 0..y.len() {
            y[i] = ((i % 3) as f32) - 1.0;
        }
        let mut bp = [0i32; 2];
        find_best_pitch(&xcorr, &y, 4, 8, &mut bp);
        assert_eq!(bp[0], 3);
        assert!(bp[1] < 8);
    }

    /// `pitch_search` on a periodic signal recovers a lag close to the period.
    #[test]
    fn pitch_search_finds_period() {
        let len = 256usize;
        let period = 40usize;
        let mut x_lp = vec![0.0f32; len];
        for i in 0..len {
            x_lp[i] = ((i * 7 % 11) as f32) * 0.01;
        }
        // Build y so that y[i + period] == x_lp[i] dominates.
        let max_pitch = 100i32;
        let mut y = vec![0.0f32; len + max_pitch as usize + 8];
        for i in 0..len {
            y[i] = ((i * 7 % 11) as f32) * 0.01;
        }
        for i in 0..max_pitch as usize {
            y[i + period] += x_lp[i.min(len - 1)];
        }
        let mut pitch = 0i32;
        pitch_search(&x_lp, &y, len as i32, max_pitch, &mut pitch, 4);
        assert!(pitch > 0 && pitch <= max_pitch);
    }

    /// `remove_doubling` on a strongly periodic signal returns a period close
    /// to the true one without panicking.
    #[test]
    fn remove_doubling_periodic() {
        let maxperiod = 1024i32;
        let n = 960i32;
        let period = 200usize;
        let len = (maxperiod + n) as usize;
        let mut buf = vec![0.0f32; len];
        for i in 0..len {
            let phase = i % period;
            buf[i] = if phase < period / 2 { 1.0 } else { -1.0 };
        }
        let mut t0 = period as i32;
        let g = remove_doubling(&buf, maxperiod, 15, n, &mut t0, period as i32, 0.5, 4);
        assert!(g.is_finite());
        assert!(t0 > 0);
        assert!(t0 <= maxperiod);
    }
}

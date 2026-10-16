/* Copyright (c) 2009-2010 Xiph.Org Foundation
   Written by Jean-Marc Valin */
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

//! Transliteration of `celt/celt_lpc.h` and `celt/celt_lpc.c` (libopus 1.6.1)
//! into safe Rust.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! Configuration (pinned reference build, float + x86 RTCD at arch 4):
//!
//! * `FIXED_POINT` is undefined, so all `#ifdef FIXED_POINT` blocks are
//!   dropped: `_celt_lpc` writes the coefficients directly into `_lpc`
//!   (`float *lpc = _lpc`) and its bandwidth-expansion/`Q12` conversion
//!   epilogue does not exist; the `opus_val32`/`opus_val16` distinction is a
//!   no-op.
//! * `celt_fir_sse4_1` only exists when `FIXED_POINT` is defined
//!   (`celt/x86/celt_lpc_sse.h:35`), and `nm reference/lib-ref/libopus.a`
//!   confirms it: there is a `T celt_fir_c` and **no** `celt_fir_sse4_1`.
//!   `OVERRIDE_CELT_FIR` is therefore never defined and `celt_fir` *is*
//!   `celt_fir_c`.  Its inner `xcorr_kernel` macro still expands to
//!   `xcorr_kernel_sse`, so [`celt_fir`] delegates to
//!   [`crate::celt::kernels::celt_fir`] (which is exactly that
//!   implementation).
//! * `celt_iir` has no SIMD variant in any configuration; the C loop is
//!   transliterated verbatim.
//! * `_celt_autocorr` calls `celt_pitch_xcorr`, which *is* runtime-dispatched
//!   (`PITCH_XCORR_IMPL`); when called with `arch == 4` it selects
//!   `celt_pitch_xcorr_avx2`, i.e. [`crate::celt::kernels::celt_pitch_xcorr`].
//!   The `arch` argument is kept in the signature (SILK's
//!   `silk/fixed/autocorr_FIX.c` and `celt/pitch.c` thread it through) and is
//!   accepted but ignored by the kernel layer, which has no `arch` parameter
//!   any more — see the module docs of [`crate::celt::kernels`].
//! * `window` is `const celt_coef *`, i.e. a pointer that the callers pass as
//!   `NULL` when `overlap == 0`; it becomes `Option<&[celt_coef]>`.
//!
//! Index conventions: [`celt_fir`] reproduces the C "reads `x[i+j-ord]`"
//! idiom, so the `x` slice must start `ord` elements *before* the C `x`
//! pointer (`x_rust[i] == x_c[i - ord]`).  This is the same convention as
//! [`crate::celt::kernels::celt_fir`].  The aliasing in
//! `celt_decoder.c:970`, where the C passes the same buffer as `_x` and `_y`,
//! cannot be expressed with `&[T]`/`&mut [T]`; a Rust caller must copy `_x`
//! first (the arithmetic is unaffected).

use super::arch::*;
use super::kernels;
use super::mathops::frac_div32;

/// `CELT_LPC_ORDER` (`celt/celt_lpc.h:38`).
pub const CELT_LPC_ORDER: i32 = 24;

/// `celt/celt_lpc.c:37` `_celt_lpc`.
///
/// Levinson-Durbin recursion.  In the float build `lpc` is the caller's
/// output buffer (`float *lpc = _lpc`) and is zeroed first.
///
/// `_celt_lpc` is called from SILK-adjacent code as well as from
/// `celt_decoder.c` and `celt/pitch.c`, so the name, parameter list and `pub`
/// visibility are kept exactly as in C.
pub fn _celt_lpc(lpc: &mut [opus_val16], ac: &[opus_val32], p: i32) {
    debug_assert!(p >= 0);
    debug_assert!(lpc.len() >= p as usize);
    debug_assert!(ac.len() >= p as usize + 1);
    let mut error: opus_val32 = ac[0];
    // OPUS_CLEAR(lpc, p)
    for i in 0..p as usize {
        lpc[i] = 0.;
    }
    if ac[0] > 1e-10f32 {
        for i in 0..p as usize {
            /* Sum up this iteration's reflection coefficient */
            let mut rr: opus_val32 = 0.;
            for j in 0..i {
                rr += MULT32_32_Q31(lpc[j], ac[i - j]);
            }
            rr += SHR32(ac[i + 1], 6);
            let r = -frac_div32(SHL32(rr, 6), error);
            /*  Update LPC coefficients and total error */
            lpc[i] = SHR32(r, 6);
            for j in 0..((i + 1) >> 1) {
                let tmp1 = lpc[j];
                let tmp2 = lpc[i - 1 - j];
                lpc[j] = tmp1 + MULT32_32_Q31(r, tmp2);
                lpc[i - 1 - j] = tmp2 + MULT32_32_Q31(r, tmp1);
            }

            error = error - MULT32_32_Q31(MULT32_32_Q31(r, r), error);
            /* Bail out once we get 30 dB gain */
            if error <= 0.001f32 * ac[0] {
                break;
            }
        }
    }
}

/// `celt/celt_lpc.c:146` `celt_fir_c` (the live `celt_fir` in this
/// configuration: no `celt_fir_sse4_1`, no `OVERRIDE_CELT_FIR`).
///
/// # Slice lengths / index convention
/// The C reads `x[i+j-ord]`, so `x` must start `ord` elements before the C `x`
/// pointer: `x_rust[i] == x_c[i - ord]`, `x_c[k] == x_rust[k + ord]`.  With
/// that convention `x.len() >= N + ord` covers the `xcorr_kernel` over-read;
/// `num.len() >= ord` and `y.len() >= N`.  `arch` is accepted and ignored, as
/// the `xcorr_kernel` macro discards it.
pub fn celt_fir_c(
    x: &[opus_val16],
    num: &[opus_val16],
    y: &mut [opus_val16],
    n: i32,
    ord: i32,
    _arch: i32,
) {
    celt_assert!(x.as_ptr() != y.as_ptr() as *const opus_val16);
    kernels::celt_fir(x, num, y, n, ord);
}

/// The `celt_fir(x, num, y, N, ord, arch)` macro (`celt/celt_lpc.h:51-52`):
/// expands to [`celt_fir_c`].
pub fn celt_fir(
    x: &[opus_val16],
    num: &[opus_val16],
    y: &mut [opus_val16],
    n: i32,
    ord: i32,
    arch: i32,
) {
    celt_fir_c(x, num, y, n, ord, arch);
}

/// `celt/celt_lpc.c:194` `celt_iir`.
///
/// No SIMD variant exists in this configuration (`SMALL_FOOTPRINT` is
/// undefined, so the unrolled-by-4 branch is the live one).  It calls
/// `xcorr_kernel(rden, y+i, sum, ord, arch)` — the SSE kernel — and then
/// patches up the four results.
///
/// `den.len() >= ord`, `ord >= 1` and `ord % 4 == 0` (the C asserts
/// `(ord&3)==0`), `mem.len() >= ord` and is read *and* written (`y[i] =
/// -mem[ord-i-1]` first, `mem[i] = _y[N-i-1]` last), `x.len() >= N`,
/// `y.len() >= N`.
///
/// The final `mem[i] = _y[N-i-1]` loop requires **`N >= ord`**: for `N < ord`
/// the C reads `_y[-1]`, `_y[-2]`, ... (out of bounds).  The decoder's
/// `extrapolation_len` always satisfies this.
pub fn celt_iir(
    x: &[opus_val32],
    den: &[opus_val16],
    y: &mut [opus_val32],
    n: i32,
    ord: i32,
    mem: &mut [opus_val16],
    _arch: i32,
) {
    debug_assert!((ord & 3) == 0, "celt_iir: ord must be a multiple of 4");
    debug_assert!(n >= 0);
    let nn = n as usize;
    let oo = ord as usize;
    debug_assert!(den.len() >= oo);
    debug_assert!(mem.len() >= oo);
    debug_assert!(x.len() >= nn);
    debug_assert!(y.len() >= nn);
    debug_assert!(nn >= oo, "celt_iir: the C tail reads _y[N-i-1], i < ord");
    let mut rden = vec![0 as opus_val16; oo];
    let mut yy = vec![0 as opus_val16; nn + oo];
    for i in 0..oo {
        rden[i] = den[oo - i - 1];
    }
    for i in 0..oo {
        yy[i] = -mem[oo - i - 1];
    }
    for i in oo..nn + oo {
        yy[i] = 0.;
    }
    let mut i: i32 = 0;
    while i < n - 3 {
        let ii = i as usize;
        /* Unroll by 4 as if it were an FIR filter */
        let mut sum = [x[ii], x[ii + 1], x[ii + 2], x[ii + 3]];
        kernels::xcorr_kernel(&rden, &yy[ii..], &mut sum, ord);
        /* Patch up the result to compensate for the fact that this is an IIR */
        yy[ii + oo] = -SROUND16(sum[0], SIG_SHIFT);
        y[ii] = sum[0];
        sum[1] = MAC16_16(sum[1], yy[ii + oo], den[0]);
        yy[ii + oo + 1] = -SROUND16(sum[1], SIG_SHIFT);
        y[ii + 1] = sum[1];
        sum[2] = MAC16_16(sum[2], yy[ii + oo + 1], den[0]);
        sum[2] = MAC16_16(sum[2], yy[ii + oo], den[1]);
        yy[ii + oo + 2] = -SROUND16(sum[2], SIG_SHIFT);
        y[ii + 2] = sum[2];

        sum[3] = MAC16_16(sum[3], yy[ii + oo + 2], den[0]);
        sum[3] = MAC16_16(sum[3], yy[ii + oo + 1], den[1]);
        sum[3] = MAC16_16(sum[3], yy[ii + oo], den[2]);
        yy[ii + oo + 3] = -SROUND16(sum[3], SIG_SHIFT);
        y[ii + 3] = sum[3];
        i += 4;
    }
    while i < n {
        let ii = i as usize;
        let mut sum = x[ii];
        for j in 0..oo {
            sum -= MULT16_16(rden[j], yy[ii + j]);
        }
        yy[ii + oo] = SROUND16(sum, SIG_SHIFT);
        y[ii] = sum;
        i += 1;
    }
    for i in 0..oo {
        mem[i] = y[nn - i - 1];
    }
}

/// `celt/celt_lpc.c:284` `_celt_autocorr`.
///
/// In this configuration (`FIXED_POINT` undefined) `shift` is set to 0 and
/// never changed: the whole fixed-point `ac0` scaling block and the final
/// renormalisation are dropped, and the function always returns 0.  The
/// `window` pointer is `NULL` when `overlap == 0` and non-`NULL` otherwise;
/// `celt_coef` is `float`, so `COEF2VAL16` is the identity.
///
/// `x.len() >= n`, `ac.len() >= lag + 1`; with `overlap > 0`,
/// `window` must be `Some` with at least `overlap` elements.
///
/// The first `celt_pitch_xcorr` call is the runtime-dispatched
/// `PITCH_XCORR_IMPL[arch & OPUS_ARCHMASK]`, i.e.
/// [`kernels::celt_pitch_xcorr`] for `arch == 4`.
pub fn _celt_autocorr(
    x: &[opus_val16],
    ac: &mut [opus_val32],
    window: Option<&[celt_coef]>,
    overlap: i32,
    lag: i32,
    n: i32,
    _arch: i32,
) -> i32 {
    let fast_n = n - lag;
    debug_assert!(n > 0);
    debug_assert!(overlap >= 0);
    debug_assert!(x.len() >= n as usize);
    debug_assert!(ac.len() >= (lag + 1) as usize);
    let nn = n as usize;
    let xx_storage;
    let xptr: &[opus_val16] = if overlap == 0 {
        x
    } else {
        let mut xx = vec![0 as opus_val16; nn];
        for i in 0..nn {
            xx[i] = x[i];
        }
        for i in 0..overlap as usize {
            let w = COEF2VAL16(window.expect("_celt_autocorr: window required")[i]);
            xx[i] = MULT16_16_Q15(x[i], w);
            xx[nn - i - 1] = MULT16_16_Q15(x[nn - i - 1], w);
        }
        xx_storage = xx;
        &xx_storage
    };
    let shift = 0;
    kernels::celt_pitch_xcorr(xptr, xptr, ac, fast_n, lag + 1);
    for k in 0..=lag as usize {
        let mut d: opus_val32 = 0.;
        let mut i = k + fast_n as usize;
        while i < nn {
            d = MAC16_16(d, xptr[i], xptr[i - k]);
            i += 1;
        }
        ac[k] += d;
    }
    shift
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `_celt_lpc` with a zero autocorrelation returns all-zero coefficients.
    #[test]
    fn lpc_zero_ac() {
        let ac = [0.0f32; CELT_LPC_ORDER as usize + 1];
        let mut lpc = [7.0f32; CELT_LPC_ORDER as usize];
        _celt_lpc(&mut lpc, &ac, CELT_LPC_ORDER);
        assert!(lpc.iter().all(|&v| v == 0.0));
    }

    /// A tiny autocorrelation: `ac[0]` is the total energy of `x`.
    #[test]
    fn autocorr_energy_no_window() {
        let x = [1.0f32, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0];
        let mut ac = [0.0f32; 5];
        let shift = _celt_autocorr(&x, &mut ac, None, 0, 4, x.len() as i32, 4);
        assert_eq!(shift, 0);
        let expect0: f32 = x.iter().map(|v| v * v).sum();
        assert!((ac[0] - expect0).abs() < 1e-3);
        assert!(ac[1..].iter().all(|v| v.is_finite()));
    }

    /// `_celt_autocorr` with a constant window of 1 must equal the unwindowed
    /// result exactly.
    #[test]
    fn autocorr_window_identity() {
        let x = [0.5f32, -0.25, 0.125, 2.0, -3.0, 0.75, -1.5, 0.0];
        let ac_no = {
            let mut ac = [0.0f32; 3];
            _celt_autocorr(&x, &mut ac, None, 0, 2, x.len() as i32, 4);
            ac
        };
        let w = [1.0f32; 3];
        let ac_w = {
            let mut ac = [0.0f32; 3];
            _celt_autocorr(&x, &mut ac, Some(&w), 3, 2, x.len() as i32, 4);
            ac
        };
        for k in 0..3 {
            assert_eq!(ac_no[k].to_bits(), ac_w[k].to_bits());
        }
    }

    /// `celt_fir` with a single tap equal to one computes
    /// `y[i] = x_c[i] + x_c[i-1]`, checked with the `ord`-element slice offset
    /// (`x_rust[k] == x_c[k - ord]`).
    #[test]
    fn fir_single_tap() {
        let n = 16usize;
        let ord = 1usize;
        let mut x = vec![0.0f32; n + ord + 4];
        for (i, v) in x.iter_mut().enumerate() {
            *v = i as f32 - 2.0;
        }
        let num = [1.0f32];
        let mut y = vec![0.0f32; n];
        celt_fir(&x, &num, &mut y, n as i32, ord as i32, 4);
        for i in 0..n {
            // x_c[i] == x[i + ord], x_c[i - 1] == x[i + ord - 1]
            let expect = x[i + ord] + x[i + ord - 1];
            assert_eq!(y[i].to_bits(), expect.to_bits());
        }
    }

    /// `celt_iir` with a zero denominator is the identity, and the memory
    /// update stores the last `ord` outputs.
    #[test]
    fn iir_identity() {
        let n = 8usize;
        let ord = 4usize;
        let x: Vec<f32> = (0..n).map(|i| i as f32 + 0.5).collect();
        let den = [0.0f32; 4];
        let mut mem = [0.25f32; 4];
        let mut y = vec![0.0f32; n];
        celt_iir(&x, &den, &mut y, n as i32, ord as i32, &mut mem, 4);
        for i in 0..n {
            assert_eq!(y[i].to_bits(), x[i].to_bits());
        }
        for i in 0..ord {
            assert_eq!(mem[i].to_bits(), y[n - i - 1].to_bits());
        }
    }
}

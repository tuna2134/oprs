/* Copyright (c) 2007-2008 CSIRO
   Copyright (c) 2007-2008 Xiph.Org Foundation
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

//! Transliteration of `celt/mdct.c` and `celt/mdct.h`.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! Only the branches active in the pinned reference configuration are ported:
//! `FIXED_POINT` undefined (so no `fft_downshift`/`headroom`/`pre_shift`
//! machinery and no `celt_cos_norm`), `HAVE_ARM_NE10` undefined (no
//! `CLT_MDCT_FORWARD_IMPL` dispatch, so the `clt_mdct_forward` /
//! `clt_mdct_backward` macros name the `_c` functions) and `CUSTOM_MODES`
//! defined.
//!
//! The C code uses the caller's `out` buffer as scratch for the complex FFT
//! (`(kiss_fft_cpx*)(out+(overlap>>1))`).  Rather than reinterpreting a
//! `&mut [f32]` as `&mut [kiss_fft_cpx]`, the port keeps that scratch in a
//! separate `Vec<kiss_fft_cpx>`.  The two are equivalent: the pre-rotation
//! writes every element before the FFT reads it, the post-rotation writes every
//! element of the region exactly once, and every alias (the odd-N4 middle pair
//! that C computes twice) is read before it is written.

use crate::celt::arch::*;
use crate::celt::kiss_fft::*;
use crate::celt::mathops::PI;

/// `mdct_lookup`
pub struct mdct_lookup {
    pub n: i32,
    pub maxshift: i32,
    /// `const kiss_fft_state *kfft[4]`
    pub kfft: [Option<Box<kiss_fft_state>>; 4],
    /// `const kiss_twiddle_scalar * OPUS_RESTRICT trig`
    pub trig: Vec<kiss_twiddle_scalar>,
}

impl Default for mdct_lookup {
    fn default() -> Self {
        mdct_lookup {
            n: 0,
            maxshift: 0,
            kfft: [None, None, None, None],
            trig: Vec::new(),
        }
    }
}

/// `clt_mdct_init()`
pub fn clt_mdct_init(l: &mut mdct_lookup, N: i32, maxshift: i32, arch: i32) -> i32 {
    let mut N2 = N >> 1;
    let mut N = N;
    l.n = N;
    l.maxshift = maxshift;
    for i in 0..=maxshift {
        let st = if i == 0 {
            opus_fft_alloc(N >> 2 >> i, arch)
        } else {
            opus_fft_alloc_twiddles(N >> 2 >> i, l.kfft[0].as_deref(), arch)
        };
        /* ENABLE_TI_DSPLIB55 is not defined, so the NULL check is active. */
        if st.is_none() {
            return 0;
        }
        l.kfft[i as usize] = st;
    }
    l.trig = vec![0.0f32; (N - (N2 >> maxshift)) as usize];
    let mut tpos = 0usize;
    for _shift in 0..=maxshift {
        /* We have enough points that sine isn't necessary. */
        for i in 0..N2 {
            /* FIXED_POINT/ENABLE_QEXT are undefined: the double `cos` branch.
               C: `trig[i] = (kiss_twiddle_scalar)cos(2*PI*(i+.125)/N);` */
            l.trig[tpos + i as usize] =
                (2.0 * PI * (i as f64 + 0.125) / N as f64).cos() as kiss_twiddle_scalar;
        }
        tpos += N2 as usize;
        N2 >>= 1;
        N >>= 1;
    }
    1
}

/// `clt_mdct_clear()`
pub fn clt_mdct_clear(l: &mut mdct_lookup, arch: i32) {
    for i in 0..=l.maxshift {
        opus_fft_free(l.kfft[i as usize].take(), arch);
    }
    l.trig = Vec::new();
}

/* Forward MDCT trashes the input array.  (The current C implementation only
   reads `in`, so the Rust port takes it immutably; a mutable slice coerces.) */
/// `clt_mdct_forward_c()`
pub fn clt_mdct_forward_c(
    l: &mdct_lookup,
    input: &[kiss_fft_scalar],
    out: &mut [kiss_fft_scalar],
    window: &[celt_coef],
    overlap: i32,
    shift: i32,
    stride: i32,
    arch: i32,
) {
    let _ = arch;
    let st = l.kfft[shift as usize]
        .as_deref()
        .expect("clt_mdct_forward_c: kfft[shift] must have been initialised");
    /* FIXED_POINT is not defined. */
    let scale: celt_coef = st.scale;

    let mut N = l.n;
    let mut trig_off = 0i32;
    for _i in 0..shift {
        N >>= 1;
        trig_off += N;
    }
    let N2 = N >> 1;
    let N4 = N >> 2;
    let trig = &l.trig[trig_off as usize..];

    let mut f = vec![0.0f32; N2 as usize];
    let mut f2 = vec![kiss_fft_cpx::default(); N4 as usize];

    /* Consider the input to be composed of four blocks: [a, b, c, d] */
    /* Window, shuffle, fold */
    {
        /* The cursors are `isize` because C lets them walk one element past the
           end of their buffer on the final decrement; the out-of-range value is
           never dereferenced. */
        let mut xp1: isize = (overlap >> 1) as isize;
        let mut xp2: isize = (N2 - 1 + (overlap >> 1)) as isize;
        let mut yp: usize = 0;
        let mut wp1: isize = (overlap >> 1) as isize;
        let mut wp2: isize = ((overlap >> 1) - 1) as isize;
        let lim1 = (overlap + 3) >> 2;
        for _ in 0..lim1 {
            /* Real part arranged as -d-cR, Imag part arranged as -b+aR*/
            f[yp] = S_MUL(input[(xp1 + N2 as isize) as usize], window[wp2 as usize])
                + S_MUL(input[xp2 as usize], window[wp1 as usize]);
            yp += 1;
            f[yp] = S_MUL(input[xp1 as usize], window[wp1 as usize])
                - S_MUL(input[(xp2 - N2 as isize) as usize], window[wp2 as usize]);
            yp += 1;
            xp1 += 2;
            xp2 -= 2;
            wp1 += 2;
            wp2 -= 2;
        }
        wp1 = 0;
        wp2 = (overlap - 1) as isize;
        for _ in lim1..(N4 - lim1) {
            /* Real part arranged as a-bR, Imag part arranged as -c-dR */
            f[yp] = input[xp2 as usize];
            yp += 1;
            f[yp] = input[xp1 as usize];
            yp += 1;
            xp1 += 2;
            xp2 -= 2;
        }
        /* C: `for(;i<N4-((overlap+3)>>2);i++)` followed by `for(;i<N4;i++)`.
           After the first two loops the index is max(lim1, N4-lim1), which is
           what the third loop starts from. */
        let i3 = if lim1 > N4 - lim1 { lim1 } else { N4 - lim1 };
        for _ in i3..N4 {
            /* Real part arranged as a-bR, Imag part arranged as -c-dR */
            f[yp] = -S_MUL(input[(xp1 - N2 as isize) as usize], window[wp1 as usize])
                + S_MUL(input[xp2 as usize], window[wp2 as usize]);
            yp += 1;
            f[yp] = S_MUL(input[xp1 as usize], window[wp2 as usize])
                + S_MUL(input[(xp2 + N2 as isize) as usize], window[wp1 as usize]);
            yp += 1;
            xp1 += 2;
            xp2 -= 2;
            wp1 += 2;
            wp2 -= 2;
        }
    }
    /* Pre-rotation */
    {
        let mut yp: usize = 0;
        for i in 0..N4 as usize {
            let t0 = trig[i];
            let t1 = trig[N4 as usize + i];
            let re = f[yp];
            let im = f[yp + 1];
            yp += 2;
            let yr = S_MUL(re, t0) - S_MUL(im, t1);
            let yi = S_MUL(im, t0) + S_MUL(re, t1);
            /* ENABLE_QEXT is not defined, so the scaling happens after the
               pre-rotation (for floating point it does not matter). */
            let yc = kiss_fft_cpx {
                r: S_MUL2(yr, scale),
                i: S_MUL2(yi, scale),
            };
            f2[st.bitrev[i] as usize] = yc;
        }
    }

    /* N/4 complex FFT, does not downscale anymore */
    opus_fft_impl(st, &mut f2);

    /* Post-rotate */
    {
        let mut fp: usize = 0;
        let mut yp1: isize = 0;
        let mut yp2: isize = (stride * (N2 - 1)) as isize;
        for i in 0..N4 as usize {
            /* `headroom` exists only in a fixed-point build; PSHR32 is the
               identity here, so the discarded argument is 0. */
            let t0 = trig[i];
            let t1 = trig[N4 as usize + i];
            let yr = PSHR32(S_MUL(f2[fp].i, t1) - S_MUL(f2[fp].r, t0), 0);
            let yi = PSHR32(S_MUL(f2[fp].r, t1) + S_MUL(f2[fp].i, t0), 0);
            out[yp1 as usize] = yr;
            out[yp2 as usize] = yi;
            fp += 1;
            yp1 += 2 * stride as isize;
            yp2 -= 2 * stride as isize;
        }
    }
}

/// `clt_mdct_forward()` — the C macro names `clt_mdct_forward_c` here.
#[inline(always)]
pub fn clt_mdct_forward(
    l: &mdct_lookup,
    input: &[kiss_fft_scalar],
    out: &mut [kiss_fft_scalar],
    window: &[celt_coef],
    overlap: i32,
    shift: i32,
    stride: i32,
    arch: i32,
) {
    clt_mdct_forward_c(l, input, out, window, overlap, shift, stride, arch)
}

/// `clt_mdct_backward_c()`
pub fn clt_mdct_backward_c(
    l: &mdct_lookup,
    input: &[kiss_fft_scalar],
    out: &mut [kiss_fft_scalar],
    window: &[celt_coef],
    overlap: i32,
    shift: i32,
    stride: i32,
    arch: i32,
) {
    let _ = arch;
    let mut N = l.n;
    let mut trig_off = 0i32;
    for _i in 0..shift {
        N >>= 1;
        trig_off += N;
    }
    let N2 = N >> 1;
    let N4 = N >> 2;
    let trig = &l.trig[trig_off as usize..];
    let kfft = l.kfft[shift as usize]
        .as_deref()
        .expect("clt_mdct_backward_c: kfft[shift] must have been initialised");

    /* FIXED_POINT is not defined: pre_shift = fft_shift = post_shift = 0. */
    let pre_shift: i32 = 0;
    let post_shift: i32 = 0;
    /* `ARG_FIXED(fft_shift)` in the opus_fft_impl() call below is empty here. */

    let mut f2 = vec![kiss_fft_cpx::default(); N4 as usize];

    /* Pre-rotate */
    {
        let mut xp1: isize = 0;
        let mut xp2: isize = (stride * (N2 - 1)) as isize;
        let t = trig;
        let bitrev = &kfft.bitrev;
        for i in 0..N4 as usize {
            let rev = bitrev[i] as usize;
            let x1 = SHL32_ovflw(input[xp1 as usize], pre_shift);
            let x2 = SHL32_ovflw(input[xp2 as usize], pre_shift);
            let yr = ADD32_ovflw(S_MUL(x2, t[i]), S_MUL(x1, t[N4 as usize + i]));
            let yi = SUB32_ovflw(S_MUL(x1, t[i]), S_MUL(x2, t[N4 as usize + i]));
            /* We swap real and imag because we use an FFT instead of an IFFT.
               Storing the pre-rotation directly in the bitrev order. */
            f2[rev].r = yi;
            f2[rev].i = yr;
            xp1 += 2 * stride as isize;
            xp2 -= 2 * stride as isize;
        }
    }

    opus_fft_impl(kfft, &mut f2);

    /* Post-rotate and de-shuffle from both ends of the buffer at once to make
       it in-place. */
    {
        let overlap2 = (overlap >> 1) as usize;
        let mut yp0: usize = overlap2;
        let mut yp1: usize = overlap2 + N2 as usize - 2;
        let t = trig;
        /* Loop to (N4+1)>>1 to handle odd N4.  When N4 is odd, the middle pair
           will be computed twice. */
        for i in 0..((N4 + 1) >> 1) as usize {
            /* We swap real and imag because we're using an FFT instead of an
               IFFT. */
            let mut re = f2[i].i;
            let mut im = f2[i].r;
            let t0 = t[i];
            let t1 = t[N4 as usize + i];
            /* We'd scale up by 2 here, but instead it's done when mixing the
               windows. */
            let yr = PSHR32_ovflw(ADD32_ovflw(S_MUL(re, t0), S_MUL(im, t1)), post_shift);
            let yi = PSHR32_ovflw(SUB32_ovflw(S_MUL(re, t1), S_MUL(im, t0)), post_shift);
            /* We swap real and imag because we're using an FFT instead of an
               IFFT. */
            let mid = (N4 - 1 - i as i32) as usize;
            re = f2[mid].i;
            im = f2[mid].r;
            out[yp0] = yr;
            out[yp1 + 1] = yi;

            let t0 = t[(N4 - i as i32 - 1) as usize];
            let t1 = t[(N2 - i as i32 - 1) as usize];
            /* We'd scale up by 2 here, but instead it's done when mixing the
               windows. */
            let yr = PSHR32_ovflw(ADD32_ovflw(S_MUL(re, t0), S_MUL(im, t1)), post_shift);
            let yi = PSHR32_ovflw(SUB32_ovflw(S_MUL(re, t1), S_MUL(im, t0)), post_shift);
            out[yp1] = yr;
            out[yp0 + 1] = yi;
            yp0 += 2;
            yp1 -= 2;
        }
    }

    /* Mirror on both sides for TDAC */
    {
        let ov = overlap as usize;
        for i in 0..(overlap / 2) as usize {
            let x1 = out[ov - 1 - i];
            let x2 = out[i];
            out[i] = SUB32_ovflw(S_MUL(x2, window[ov - 1 - i]), S_MUL(x1, window[i]));
            out[ov - 1 - i] =
                ADD32_ovflw(S_MUL(x2, window[i]), S_MUL(x1, window[ov - 1 - i]));
        }
    }
}

/// `clt_mdct_backward()` — the C macro names `clt_mdct_backward_c` here.
#[inline(always)]
pub fn clt_mdct_backward(
    l: &mdct_lookup,
    input: &[kiss_fft_scalar],
    out: &mut [kiss_fft_scalar],
    window: &[celt_coef],
    overlap: i32,
    shift: i32,
    stride: i32,
    arch: i32,
) {
    clt_mdct_backward_c(l, input, out, window, overlap, shift, stride, arch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window CELT builds for a given overlap (`modes.c`), with the same
    /// double-precision `sin` expressions.
    fn celt_window(overlap: usize) -> Vec<f32> {
        let mut w = vec![0.0f32; overlap];
        for (i, wi) in w.iter_mut().enumerate() {
            let s = (0.5 * core::f64::consts::PI * (i as f64 + 0.5) / overlap as f64).sin();
            let s = s * s;
            *wi = (0.5 * core::f64::consts::PI * s).sin() as f32;
        }
        w
    }

    fn rng(seed: u32) -> impl FnMut() -> f32 {
        let mut s = seed;
        move || {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            (s as i32) as f32 / 131072.0
        }
    }

    #[test]
    fn mdct_init_structure() {
        let mut l = mdct_lookup::default();
        assert_eq!(clt_mdct_init(&mut l, 1920, 3, 0), 1);
        assert_eq!(l.n, 1920);
        assert_eq!(l.maxshift, 3);
        assert_eq!(l.trig.len(), 1920 - (960 >> 3));
        for (i, expect) in [(0, 480), (1, 240), (2, 120), (3, 60)] {
            let st = l.kfft[i].as_ref().expect("kfft");
            assert_eq!(st.nfft, expect);
            assert_eq!(st.scale, 1.0f32 / expect as f32);
        }
        assert_eq!(l.kfft[0].as_ref().unwrap().shift, -1);
        assert_eq!(l.kfft[1].as_ref().unwrap().shift, 1);
        assert_eq!(l.kfft[3].as_ref().unwrap().shift, 3);
        /* subtle: kfft[i] shares the base's twiddle table */
        assert_eq!(
            l.kfft[3].as_ref().unwrap().twiddles.len(),
            l.kfft[0].as_ref().unwrap().twiddles.len()
        );
        clt_mdct_clear(&mut l, 0);
        assert!(l.kfft[0].is_none());
        assert!(l.trig.is_empty());
    }

    #[test]
    fn mdct_init_rejects_unsupported_size() {
        /* 1920/7 is not 2,3,5-smooth -> kf_factor fails -> clt_mdct_init: 0 */
        let mut l = mdct_lookup::default();
        assert_eq!(clt_mdct_init(&mut l, 28, 0, 0), 0);
        assert!(l.kfft[0].is_none());
    }

    #[test]
    fn mdct_forward_backward_sanity() {
        const N: i32 = 1920;
        const OVERLAP: i32 = 120;
        let mut l = mdct_lookup::default();
        assert_eq!(clt_mdct_init(&mut l, N, 3, 0), 1);
        let w = celt_window(OVERLAP as usize);
        for shift in 0..=3i32 {
            let ne = (N >> shift) as usize;
            let n2 = ne / 2;
            let mut next = rng(0x1234_5678 + shift as u32);
            let input: Vec<f32> = (0..n2 + OVERLAP as usize).map(|_| next()).collect();
            let mut spec = vec![0.0f32; n2];
            clt_mdct_forward_c(&l, &input, &mut spec, &w, OVERLAP, shift, 1, 0);
            assert!(spec.iter().all(|x| x.is_finite()));
            assert!(spec.iter().any(|x| *x != 0.0));

            let mut out = vec![0.0f32; ne + OVERLAP as usize];
            clt_mdct_backward_c(&l, &spec, &mut out, &w, OVERLAP, shift, 1, 0);
            assert!(out.iter().all(|x| x.is_finite()));
            assert!(out[..n2 + (OVERLAP / 2) as usize].iter().any(|x| *x != 0.0));
        }
    }

    #[test]
    fn mdct_tdac_reconstruction() {
        /* Consecutive frames overlap-add to the original signal (TDAC).
           `clt_mdct_backward_c` consumes the previous frame's tail from
           `out[0..overlap/2)` and leaves the next frame's tail at
           `out[n2..n2+overlap/2)`, so the decoder's per-frame `OPUS_MOVE` by the
           frame size is reproduced here by copying that tail to the front. */
        const N: i32 = 480;
        const OVERLAP: i32 = 120;
        let mut l = mdct_lookup::default();
        assert_eq!(clt_mdct_init(&mut l, N, 2, 0), 1);
        let w = celt_window(OVERLAP as usize);
        let shift = 0;
        let ne = N as usize;
        let n2 = ne / 2; /* the encoder's frame size */
        let ov = OVERLAP as usize;
        let nframes = 4usize;
        let mut next = rng(0xabcd_ef01);
        let total = n2 * (nframes + 1) + ov;
        let sig: Vec<f32> = (0..total).map(|_| next()).collect();

        let mut buf = vec![0.0f32; n2 + ov];
        let mut recon = vec![0.0f32; n2 * nframes];
        for f in 0..nframes {
            if f > 0 {
                for k in 0..ov / 2 {
                    buf[k] = buf[n2 + k];
                }
            }
            let inbuf = &sig[f * n2..f * n2 + n2 + ov];
            let mut spec = vec![0.0f32; n2];
            clt_mdct_forward_c(&l, inbuf, &mut spec, &w, OVERLAP, shift, 1, 0);
            clt_mdct_backward_c(&l, &spec, &mut buf, &w, OVERLAP, shift, 1, 0);
            recon[f * n2..(f + 1) * n2].copy_from_slice(&buf[..n2]);
        }
        /* Frame 0 has no preceding tail, so compare from frame 1 on. */
        let mut maxerr = 0.0f32;
        let mut maxsig = 0.0f32;
        for i in 0..n2 * (nframes - 1) {
            maxerr = maxerr.max((recon[n2 + i] - sig[n2 + i]).abs());
            maxsig = maxsig.max(sig[n2 + i].abs());
        }
        assert!(maxerr < 1e-5 * maxsig, "TDAC reconstruction error {maxerr}");
    }
}

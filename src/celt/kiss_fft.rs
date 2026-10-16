/*Copyright (c) 2003-2004, Mark Borgerding
  Lots of modifications by Jean-Marc Valin
  Copyright (c) 2005-2007, Xiph.Org Foundation
  Copyright (c) 2008,      Xiph.Org Foundation, CSIRO

  All rights reserved.

  Redistribution and use in source and binary forms, with or without
   modification, are permitted provided that the following conditions are met:

    * Redistributions of source code must retain the above copyright notice,
       this list of conditions and the following disclaimer.
    * Redistributions in binary form must reproduce the above copyright notice,
       this list of conditions and the following disclaimer in the
       documentation and/or other materials provided with the distribution.

  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
  AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
  IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
  ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
  LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
  CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
  SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
  INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
  CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
  ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
  POSSIBILITY OF SUCH DAMAGE.*/

//! Transliteration of `celt/kiss_fft.c`, `celt/kiss_fft.h` and the part of
//! `celt/_kiss_fft_guts.h` used by the float build.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! Only the branches active in the pinned reference configuration are ported:
//! `FIXED_POINT` undefined, `USE_SIMD` undefined, `HAVE_ARM_NE10` undefined and
//! no x86 FFT kernels (there is no FFT/MDCT file under `celt/x86/`).
//! `CUSTOM_MODES` is defined, so `compute_bitrev_table`, `kf_factor`,
//! `compute_twiddles` and the allocating entry points are all active.
//! `RADIX_TWO_ONLY` is *not* defined, so radices 2, 3, 4 and 5 are all live.
//!
//! `opus_fft()`/`opus_ifft()` are macros in C that name `opus_fft_c()` /
//! `opus_ifft_c()` in this configuration (there is no ARM NE10 dispatch); both
//! spellings are provided here, the plain names forwarding to the `_c`
//! implementations.
//!
//! The C allocation out-parameters (`void *mem`, `size_t *lenmem`) have no
//! counterpart in Rust: `opus_fft_alloc*` return an owned
//! `Box<kiss_fft_state>` and `opus_fft_free` is a no-op kept for API fidelity.
//! For the same reason `shift < 0` (the state owns its twiddle table in C)
//! merely means "this is the base state" here; every state owns its own
//! `Vec<kiss_twiddle_cpx>` and a shared table is copied.  The values are the
//! same in both languages.

use crate::celt::arch::*;
use crate::types::*;

/// `kiss_fft_scalar` — `float`, since neither `USE_SIMD` nor `FIXED_POINT` is
/// defined.
pub type kiss_fft_scalar = f32;
/// `kiss_twiddle_scalar` — `float` (the `celt_coef` of a float build).
pub type kiss_twiddle_scalar = f32;

/// `kiss_fft_cpx`
#[derive(Clone, Copy, Default, PartialEq, Debug)]
#[repr(C)]
pub struct kiss_fft_cpx {
    pub r: kiss_fft_scalar,
    pub i: kiss_fft_scalar,
}

/// `kiss_twiddle_cpx`
#[derive(Clone, Copy, Default, PartialEq, Debug)]
#[repr(C)]
pub struct kiss_twiddle_cpx {
    pub r: kiss_twiddle_scalar,
    pub i: kiss_twiddle_scalar,
}

/// `MAXFACTORS`
pub const MAXFACTORS: usize = 8;

/// `arch_fft_state`
///
/// Only the ARM NE10 back-end ever allocates one of these; it is dead weight in
/// this configuration.  `void *priv` is stored as a `usize` so that
/// `kiss_fft_state` (and therefore the codec state that embeds it) stays `Send`.
pub struct arch_fft_state {
    pub is_supported: i32,
    pub priv_: usize,
}

/// `kiss_fft_state`
///
/// The C struct has an extra `int scale_shift` member between `scale` and
/// `shift` under `#ifdef FIXED_POINT`; that branch is not part of this build, so
/// the member is absent here, exactly as in the compiled reference.
pub struct kiss_fft_state {
    pub nfft: i32,
    pub scale: celt_coef,
    pub shift: i32,
    pub factors: [opus_int16; 2 * MAXFACTORS],
    /// `const opus_int16 *bitrev`
    pub bitrev: Vec<opus_int16>,
    /// `const kiss_twiddle_cpx *twiddles`
    pub twiddles: Vec<kiss_twiddle_cpx>,
    /// `arch_fft_state *arch_fft`
    pub arch_fft: Option<Box<arch_fft_state>>,
}

/* ------------------------------------------------------------------------- *
 * _kiss_fft_guts.h — the `#else /* not FIXED_POINT */` branch.
 *
 * The C macros are transliterated as `#[inline(always)]` functions with their C
 * names so that the butterflies below can be diffed against kiss_fft.c.  The
 * unused guts macros (`C_MULC`, `C_MUL4`, `C_SUBFROM`, `C_FIXDIV`, `DIVSCALAR`,
 * the fixed-point `S_MUL` family) are dropped.
 * ------------------------------------------------------------------------- */

/// `S_MUL(a,b)` — `(a)*(b)`
#[inline(always)]
pub fn S_MUL(a: kiss_fft_scalar, b: kiss_twiddle_scalar) -> kiss_fft_scalar {
    a * b
}

/// `S_MUL2(a,b)` — `(a)*(b)`
#[inline(always)]
pub fn S_MUL2(a: kiss_fft_scalar, b: kiss_fft_scalar) -> kiss_fft_scalar {
    a * b
}

/// `C_MUL(m,a,b)` — `m = a*b`
#[inline(always)]
pub fn C_MUL(a: kiss_fft_cpx, b: kiss_twiddle_cpx) -> kiss_fft_cpx {
    kiss_fft_cpx {
        r: a.r * b.r - a.i * b.i,
        i: a.r * b.i + a.i * b.r,
    }
}

/// `C_MULBYSCALAR(c,s)`
#[inline(always)]
pub fn C_MULBYSCALAR(c: kiss_fft_cpx, s: kiss_twiddle_scalar) -> kiss_fft_cpx {
    kiss_fft_cpx {
        r: c.r * s,
        i: c.i * s,
    }
}

/// `C_ADD(res,a,b)`
#[inline(always)]
pub fn C_ADD(a: kiss_fft_cpx, b: kiss_fft_cpx) -> kiss_fft_cpx {
    kiss_fft_cpx {
        r: ADD32_ovflw(a.r, b.r),
        i: ADD32_ovflw(a.i, b.i),
    }
}

/// `C_SUB(res,a,b)`
#[inline(always)]
pub fn C_SUB(a: kiss_fft_cpx, b: kiss_fft_cpx) -> kiss_fft_cpx {
    kiss_fft_cpx {
        r: SUB32_ovflw(a.r, b.r),
        i: SUB32_ovflw(a.i, b.i),
    }
}

/// `HALF_OF(x)` — `(x)*.5f`
#[inline(always)]
pub fn HALF_OF(x: kiss_fft_scalar) -> kiss_fft_scalar {
    x * 0.5f32
}

/// `KISS_FFT_COS(phase)` — `(kiss_fft_scalar) cos(phase)` (double `cos`)
#[inline(always)]
pub fn KISS_FFT_COS(phase: f64) -> kiss_fft_scalar {
    phase.cos() as kiss_fft_scalar
}

/// `KISS_FFT_SIN(phase)` — `(kiss_fft_scalar) sin(phase)` (double `sin`)
#[inline(always)]
pub fn KISS_FFT_SIN(phase: f64) -> kiss_fft_scalar {
    phase.sin() as kiss_fft_scalar
}

/// `kf_cexp(x,phase)`
#[inline(always)]
pub fn kf_cexp(x: &mut kiss_twiddle_cpx, phase: f64) {
    x.r = KISS_FFT_COS(phase);
    x.i = KISS_FFT_SIN(phase);
}

/* ------------------------------------------------------------------------- *
 * kiss_fft.c
 * ------------------------------------------------------------------------- */

/// `kf_bfly2()`
///
/// `N` is the argument the C prototype calls `N`; `opus_fft_impl` passes
/// `fstride[i]` in it, i.e. the number of butterflies, not the transform size.
fn kf_bfly2(fout: &mut [kiss_fft_cpx], m: i32, N: i32) {
    let mut fo: usize = 0;
    /* CUSTOM_MODES is defined in the reference configuration. */
    if m == 1 {
        celt_assert!(m == 1);
        for _i in 0..N {
            let t = fout[fo + 1];
            /* C_SUB( *Fout2 , *Fout , t ) */
            fout[fo + 1] = C_SUB(fout[fo], t);
            /* C_ADDTO( *Fout , t ) */
            fout[fo] = C_ADD(fout[fo], t);
            fo += 2;
        }
    } else {
        /* COEF_SHIFT is only defined in a fixed-point build; QCONST32 is the
           identity in this float build and ignores the bit count. */
        let tw: celt_coef = QCONST32(0.7071067812f32, 15);
        /* We know that m==4 here because the radix-2 is just after a radix-4 */
        celt_assert!(m == 4);
        for _i in 0..N {
            let fout2 = fo + 4;
            let t = fout[fout2];
            /* C_SUB( Fout2[0] , Fout[0] , t ); C_ADDTO( Fout[0] , t ) */
            fout[fout2] = C_SUB(fout[fo], t);
            fout[fo] = C_ADD(fout[fo], t);

            let t = kiss_fft_cpx {
                r: S_MUL(ADD32_ovflw(fout[fout2 + 1].r, fout[fout2 + 1].i), tw),
                i: S_MUL(SUB32_ovflw(fout[fout2 + 1].i, fout[fout2 + 1].r), tw),
            };
            fout[fout2 + 1] = C_SUB(fout[fo + 1], t);
            fout[fo + 1] = C_ADD(fout[fo + 1], t);

            let t = kiss_fft_cpx {
                r: fout[fout2 + 2].i,
                i: NEG32_ovflw(fout[fout2 + 2].r),
            };
            fout[fout2 + 2] = C_SUB(fout[fo + 2], t);
            fout[fo + 2] = C_ADD(fout[fo + 2], t);

            let t = kiss_fft_cpx {
                r: S_MUL(SUB32_ovflw(fout[fout2 + 3].i, fout[fout2 + 3].r), tw),
                i: S_MUL(NEG32_ovflw(ADD32_ovflw(fout[fout2 + 3].i, fout[fout2 + 3].r)), tw),
            };
            fout[fout2 + 3] = C_SUB(fout[fo + 3], t);
            fout[fo + 3] = C_ADD(fout[fo + 3], t);
            fo += 8;
        }
    }
}

/// `kf_bfly4()`
fn kf_bfly4(
    fout: &mut [kiss_fft_cpx],
    fstride: usize,
    st: &kiss_fft_state,
    m: i32,
    N: i32,
    mm: i32,
) {
    if m == 1 {
        /* Degenerate case where all the twiddles are 1. */
        let mut fo: usize = 0;
        for _i in 0..N {
            /* C_SUB( scratch0 , *Fout, Fout[2] ); C_ADDTO(*Fout, Fout[2]); */
            let scratch0 = C_SUB(fout[fo], fout[fo + 2]);
            fout[fo] = C_ADD(fout[fo], fout[fo + 2]);
            /* C_ADD( scratch1 , Fout[1] , Fout[3] ); */
            let mut scratch1 = C_ADD(fout[fo + 1], fout[fo + 3]);
            /* C_SUB( Fout[2], *Fout, scratch1 ); C_ADDTO( *Fout , scratch1 ); */
            fout[fo + 2] = C_SUB(fout[fo], scratch1);
            fout[fo] = C_ADD(fout[fo], scratch1);
            /* C_SUB( scratch1 , Fout[1] , Fout[3] ); */
            scratch1 = C_SUB(fout[fo + 1], fout[fo + 3]);

            fout[fo + 1].r = ADD32_ovflw(scratch0.r, scratch1.i);
            fout[fo + 1].i = SUB32_ovflw(scratch0.i, scratch1.r);
            fout[fo + 3].r = SUB32_ovflw(scratch0.r, scratch1.i);
            fout[fo + 3].i = ADD32_ovflw(scratch0.i, scratch1.r);
            fo += 4;
        }
    } else {
        /* m is guaranteed to be a multiple of 4. */
        let m2: i32 = 2 * m;
        let m3: i32 = 3 * m;
        for i in 0..N as usize {
            let mut fo = i * mm as usize;
            let mut tw1: usize = 0;
            let mut tw2: usize = 0;
            let mut tw3: usize = 0;
            for _j in 0..m {
                let scratch0 = C_MUL(fout[fo + m as usize], st.twiddles[tw1]);
                let scratch1 = C_MUL(fout[fo + m2 as usize], st.twiddles[tw2]);
                let scratch2 = C_MUL(fout[fo + m3 as usize], st.twiddles[tw3]);

                let scratch5 = C_SUB(fout[fo], scratch1);
                fout[fo] = C_ADD(fout[fo], scratch1);
                let scratch3 = C_ADD(scratch0, scratch2);
                let scratch4 = C_SUB(scratch0, scratch2);
                fout[fo + m2 as usize] = C_SUB(fout[fo], scratch3);
                tw1 += fstride;
                tw2 += fstride * 2;
                tw3 += fstride * 3;
                fout[fo] = C_ADD(fout[fo], scratch3);

                fout[fo + m as usize].r = ADD32_ovflw(scratch5.r, scratch4.i);
                fout[fo + m as usize].i = SUB32_ovflw(scratch5.i, scratch4.r);
                fout[fo + m3 as usize].r = SUB32_ovflw(scratch5.r, scratch4.i);
                fout[fo + m3 as usize].i = ADD32_ovflw(scratch5.i, scratch4.r);
                fo += 1;
            }
        }
    }
}

/* RADIX_TWO_ONLY is not defined in the reference configuration. */

/// `kf_bfly3()`
fn kf_bfly3(
    fout: &mut [kiss_fft_cpx],
    fstride: usize,
    st: &kiss_fft_state,
    m: i32,
    N: i32,
    mm: i32,
) {
    let m2: usize = 2 * m as usize;
    /* epi3 = st->twiddles[fstride*m] (the fixed-point branch computes it from a
       constant instead; that branch is not part of this build). */
    let epi3 = st.twiddles[fstride * m as usize];
    for i in 0..N as usize {
        let mut fo = i * mm as usize;
        let mut tw1: usize = 0;
        let mut tw2: usize = 0;
        /* For non-custom modes, m is guaranteed to be a multiple of 4. */
        /* C: `k=m; do { ... } while(--k);` — m >= 1 for every reachable case. */
        for _k in 0..m {
            let scratch1 = C_MUL(fout[fo + m as usize], st.twiddles[tw1]);
            let scratch2 = C_MUL(fout[fo + m2], st.twiddles[tw2]);

            let scratch3 = C_ADD(scratch1, scratch2);
            let mut scratch0 = C_SUB(scratch1, scratch2);
            tw1 += fstride;
            tw2 += fstride * 2;

            fout[fo + m as usize].r = SUB32_ovflw(fout[fo].r, HALF_OF(scratch3.r));
            fout[fo + m as usize].i = SUB32_ovflw(fout[fo].i, HALF_OF(scratch3.i));

            scratch0 = C_MULBYSCALAR(scratch0, epi3.i);

            fout[fo] = C_ADD(fout[fo], scratch3);

            fout[fo + m2].r = ADD32_ovflw(fout[fo + m as usize].r, scratch0.i);
            fout[fo + m2].i = SUB32_ovflw(fout[fo + m as usize].i, scratch0.r);

            fout[fo + m as usize].r = SUB32_ovflw(fout[fo + m as usize].r, scratch0.i);
            fout[fo + m as usize].i = ADD32_ovflw(fout[fo + m as usize].i, scratch0.r);

            fo += 1;
        }
    }
}

/* OVERRIDE_kf_bfly5 is not defined in the reference configuration. */

/// `kf_bfly5()`
fn kf_bfly5(
    fout: &mut [kiss_fft_cpx],
    fstride: usize,
    st: &kiss_fft_state,
    m: i32,
    N: i32,
    mm: i32,
) {
    /* ya/yb come from the twiddle table (the fixed-point branch computes them
       from constants instead; that branch is not part of this build). */
    let ya = st.twiddles[fstride * m as usize];
    let yb = st.twiddles[fstride * 2 * m as usize];
    let tw = &st.twiddles;
    let mut scratch = [kiss_fft_cpx::default(); 13];

    for i in 0..N as usize {
        let fout0 = i * mm as usize;
        let mut f0 = fout0;
        let mut f1 = fout0 + m as usize;
        let mut f2 = fout0 + 2 * m as usize;
        let mut f3 = fout0 + 3 * m as usize;
        let mut f4 = fout0 + 4 * m as usize;

        /* For non-custom modes, m is guaranteed to be a multiple of 4. */
        for u in 0..m as usize {
            scratch[0] = fout[f0];

            scratch[1] = C_MUL(fout[f1], tw[u * fstride]);
            scratch[2] = C_MUL(fout[f2], tw[2 * u * fstride]);
            scratch[3] = C_MUL(fout[f3], tw[3 * u * fstride]);
            scratch[4] = C_MUL(fout[f4], tw[4 * u * fstride]);

            scratch[7] = C_ADD(scratch[1], scratch[4]);
            scratch[10] = C_SUB(scratch[1], scratch[4]);
            scratch[8] = C_ADD(scratch[2], scratch[3]);
            scratch[9] = C_SUB(scratch[2], scratch[3]);

            fout[f0].r = ADD32_ovflw(fout[f0].r, ADD32_ovflw(scratch[7].r, scratch[8].r));
            fout[f0].i = ADD32_ovflw(fout[f0].i, ADD32_ovflw(scratch[7].i, scratch[8].i));

            scratch[5].r = ADD32_ovflw(
                scratch[0].r,
                ADD32_ovflw(S_MUL(scratch[7].r, ya.r), S_MUL(scratch[8].r, yb.r)),
            );
            scratch[5].i = ADD32_ovflw(
                scratch[0].i,
                ADD32_ovflw(S_MUL(scratch[7].i, ya.r), S_MUL(scratch[8].i, yb.r)),
            );

            scratch[6].r = ADD32_ovflw(S_MUL(scratch[10].i, ya.i), S_MUL(scratch[9].i, yb.i));
            scratch[6].i = NEG32_ovflw(ADD32_ovflw(
                S_MUL(scratch[10].r, ya.i),
                S_MUL(scratch[9].r, yb.i),
            ));

            fout[f1] = C_SUB(scratch[5], scratch[6]);
            fout[f4] = C_ADD(scratch[5], scratch[6]);

            scratch[11].r = ADD32_ovflw(
                scratch[0].r,
                ADD32_ovflw(S_MUL(scratch[7].r, yb.r), S_MUL(scratch[8].r, ya.r)),
            );
            scratch[11].i = ADD32_ovflw(
                scratch[0].i,
                ADD32_ovflw(S_MUL(scratch[7].i, yb.r), S_MUL(scratch[8].i, ya.r)),
            );
            scratch[12].r = SUB32_ovflw(S_MUL(scratch[9].i, ya.i), S_MUL(scratch[10].i, yb.i));
            scratch[12].i = SUB32_ovflw(S_MUL(scratch[10].r, yb.i), S_MUL(scratch[9].r, ya.i));

            fout[f2] = C_ADD(scratch[11], scratch[12]);
            fout[f3] = C_SUB(scratch[11], scratch[12]);

            f0 += 1;
            f1 += 1;
            f2 += 1;
            f3 += 1;
            f4 += 1;
        }
    }
}

/* CUSTOM_MODES is defined in the reference configuration. */

/// `compute_bitrev_table()`
///
/// `f` (an `opus_int16 *`) and `factors` (an `opus_int16 *`) are both passed
/// *by value* in C: a recursive call's pointer arithmetic never propagates back
/// to the caller, and every sibling sub-tree re-reads the same remaining factor
/// list.  The port therefore passes offsets by value; the write position of a
/// sibling is `f_pos + j*fstride*in_stride`, exactly as in the C loop.
/// The C function also takes a `const kiss_fft_state *st` that it never uses
/// (only a commented-out `printf` refers to it), so that parameter is dropped.
fn compute_bitrev_table(
    Fout: i32,
    f: &mut [opus_int16],
    f_pos: usize,
    fstride: usize,
    in_stride: i32,
    factors: &[opus_int16],
    fac_pos: usize,
) {
    let p: i32 = factors[fac_pos] as i32; /* the radix  */
    let m: i32 = factors[fac_pos + 1] as i32; /* stage's fft length/p */
    let fac_pos = fac_pos + 2;
    let step = fstride * in_stride as usize;

    /*printf ("fft %d %d %d %d %d %d\n", p*m, m, p, s2, fstride*in_stride, N);*/
    if m == 1 {
        for j in 0..p {
            f[f_pos + j as usize * step] = (Fout + j) as opus_int16;
        }
    } else {
        let mut Fout = Fout;
        for j in 0..p {
            compute_bitrev_table(
                Fout,
                f,
                f_pos + j as usize * step,
                fstride * p as usize,
                in_stride,
                factors,
                fac_pos,
            );
            Fout += m;
        }
    }
}

/// `kf_factor()`
///
/// `facbuf` is populated by `p1,m1,p2,m2, ...` where `p[i] * m[i] = m[i-1]` and
/// `m0 = n`.  Returns the C `int` (0 on failure, i.e. a prime factor > 5).
fn kf_factor(n: i32, facbuf: &mut [opus_int16]) -> i32 {
    let mut p: i32 = 4;
    let mut stages: i32 = 0;
    let nbak = n;
    let mut n = n;

    /* factor out powers of 4, powers of 2, then any remaining primes */
    loop {
        while n % p != 0 {
            match p {
                4 => p = 2,
                2 => p = 3,
                _ => p += 2,
            }
            /* C computes `(opus_int32)p*(opus_int32)p`, which cannot overflow
               for p <= 32000. */
            if p > 32000 || (p as i64) * (p as i64) > n as i64 {
                p = n; /* no more factors, skip to end */
            }
        }
        n /= p;
        /* RADIX_TWO_ONLY is not defined, so: `if (p>5)` */
        if p > 5 {
            return 0;
        }
        facbuf[2 * stages as usize] = p as opus_int16;
        if p == 2 && stages > 1 {
            facbuf[2 * stages as usize] = 4;
            facbuf[2] = 2;
        }
        stages += 1;
        if n <= 1 {
            break;
        }
    }
    n = nbak;
    /* Reverse the order to get the radix 4 at the end, so we can use the fast
       degenerate case.  It turns out that reversing the order also improves the
       noise behaviour. */
    for i in 0..stages / 2 {
        let tmp = facbuf[2 * i as usize];
        facbuf[2 * i as usize] = facbuf[2 * (stages - i - 1) as usize];
        facbuf[2 * (stages - i - 1) as usize] = tmp;
    }
    for i in 0..stages {
        n /= facbuf[2 * i as usize] as i32;
        facbuf[2 * i as usize + 1] = n as opus_int16;
    }
    1
}

/// `compute_twiddles()`
fn compute_twiddles(twiddles: &mut [kiss_twiddle_cpx], nfft: i32) {
    /* FIXED_POINT is not defined: the double-precision `cos`/`sin` branch. */
    let pi: f64 = 3.14159265358979323846264338327;
    for i in 0..nfft {
        let phase: f64 = (-2.0 * pi / nfft as f64) * i as f64;
        kf_cexp(&mut twiddles[i as usize], phase);
    }
}

/// `opus_fft_alloc_arch_c()`
pub fn opus_fft_alloc_arch_c(_st: &mut kiss_fft_state) -> i32 {
    0
}

/// `opus_fft_free_arch_c()`
pub fn opus_fft_free_arch_c(_st: &mut kiss_fft_state) {}

/// `opus_fft_alloc_twiddles()`
///
/// The `mem`/`lenmem` allocation protocol of the C function is replaced by an
/// owned return value, and `base == NULL` by `base == None`.
pub fn opus_fft_alloc_twiddles(
    nfft: i32,
    base: Option<&kiss_fft_state>,
    arch: i32,
) -> Option<Box<kiss_fft_state>> {
    let mut st = Box::new(kiss_fft_state {
        nfft: 0,
        scale: 0.0,
        shift: 0,
        factors: [0 as opus_int16; 2 * MAXFACTORS],
        bitrev: Vec::new(),
        twiddles: Vec::new(),
        arch_fft: None,
    });

    st.nfft = nfft;
    /* FIXED_POINT is not defined. */
    st.scale = 1.0f32 / nfft as f32;
    if let Some(base) = base {
        /* In C the twiddle pointer is merely shared with `base`; the values are
           the base table's in either case. */
        st.twiddles = base.twiddles.clone();
        st.shift = 0;
        while st.shift < 32 && nfft.wrapping_shl(st.shift as u32) != base.nfft {
            st.shift += 1;
        }
        if st.shift >= 32 {
            return None;
        }
    } else {
        let mut twiddles = vec![kiss_twiddle_cpx::default(); nfft as usize];
        compute_twiddles(&mut twiddles, nfft);
        st.twiddles = twiddles;
        st.shift = -1;
    }
    if kf_factor(nfft, &mut st.factors) == 0 {
        return None;
    }

    /* bitrev */
    st.bitrev = vec![0 as opus_int16; nfft as usize];
    compute_bitrev_table(0, &mut st.bitrev, 0, 1, 1, &st.factors, 0);

    /* Initialize architecture specific fft parameters. */
    if opus_fft_alloc_arch(&mut st, arch) != 0 {
        return None;
    }

    Some(st)
}

/// `opus_fft_alloc()`
pub fn opus_fft_alloc(nfft: i32, arch: i32) -> Option<Box<kiss_fft_state>> {
    opus_fft_alloc_twiddles(nfft, None, arch)
}

/// `opus_fft_free()`
///
/// The state is owned by its `Box` and frees itself on drop; kept for API
/// fidelity.  (In C the twiddle table is freed only when `shift < 0`, i.e. only
/// by the base state that owns it.)
pub fn opus_fft_free(cfg: Option<Box<kiss_fft_state>>, arch: i32) {
    drop(cfg);
    let _ = arch;
}

/// `opus_fft_alloc_arch(st, arch)`:
/// `((void)(arch), opus_fft_alloc_arch_c(_st))` — only the ARM NE10 back-end
/// overrides it, and that is not part of this build.
#[inline(always)]
fn opus_fft_alloc_arch(st: &mut kiss_fft_state, arch: i32) -> i32 {
    let _ = arch;
    opus_fft_alloc_arch_c(st)
}

/* `fft_downshift()` is a no-op macro when FIXED_POINT is undefined, so the C
   calls to it are dropped. */

/// `opus_fft_impl()`
pub fn opus_fft_impl(st: &kiss_fft_state, fout: &mut [kiss_fft_cpx]) {
    let mut m2: i32;
    let mut m: i32;
    let mut L: usize;
    let mut fstride = [0i32; MAXFACTORS];

    /* st->shift can be -1 */
    let shift = if st.shift > 0 { st.shift } else { 0 };

    fstride[0] = 1;
    L = 0;
    loop {
        let p = st.factors[2 * L] as i32;
        m = st.factors[2 * L + 1] as i32;
        fstride[L + 1] = fstride[L] * p;
        L += 1;
        if m == 1 {
            break;
        }
    }
    m = st.factors[2 * L - 1] as i32;
    let mut i = L as isize - 1;
    while i >= 0 {
        let iu = i as usize;
        if i != 0 {
            m2 = st.factors[2 * iu - 1] as i32;
        } else {
            m2 = 1;
        }
        match st.factors[2 * iu] as i32 {
            2 => {
                /* fft_downshift(fout, st->nfft, &downshift, 1); */
                kf_bfly2(fout, m, fstride[iu]);
            }
            4 => {
                /* fft_downshift(fout, st->nfft, &downshift, 2); */
                kf_bfly4(fout, (fstride[iu] << shift) as usize, st, m, fstride[iu], m2);
            }
            3 => {
                /* fft_downshift(fout, st->nfft, &downshift, 2); */
                kf_bfly3(fout, (fstride[iu] << shift) as usize, st, m, fstride[iu], m2);
            }
            5 => {
                /* fft_downshift(fout, st->nfft, &downshift, 3); */
                kf_bfly5(fout, (fstride[iu] << shift) as usize, st, m, fstride[iu], m2);
            }
            _ => {}
        }
        m = m2;
        i -= 1;
    }
    /* fft_downshift(fout, st->nfft, &downshift, downshift); */
}

/// `opus_fft_c()`
pub fn opus_fft_c(st: &kiss_fft_state, fin: &[kiss_fft_cpx], fout: &mut [kiss_fft_cpx]) {
    /* FIXED_POINT is not defined, so there is no scale_shift here. */
    let scale = st.scale;

    celt_assert2!(fin.as_ptr() != fout.as_ptr(), "In-place FFT not supported");
    /* Bit-reverse the input */
    for i in 0..st.nfft as usize {
        let x = fin[i];
        let rev = st.bitrev[i] as usize;
        fout[rev].r = S_MUL2(x.r, scale);
        fout[rev].i = S_MUL2(x.i, scale);
    }
    opus_fft_impl(st, fout);
}

/// `opus_fft()` — the C macro `((void)(arch), opus_fft_c(...))`.
#[inline(always)]
pub fn opus_fft(
    cfg: &kiss_fft_state,
    fin: &[kiss_fft_cpx],
    fout: &mut [kiss_fft_cpx],
    arch: i32,
) {
    let _ = arch;
    opus_fft_c(cfg, fin, fout)
}

/// `opus_ifft_c()`
pub fn opus_ifft_c(st: &kiss_fft_state, fin: &[kiss_fft_cpx], fout: &mut [kiss_fft_cpx]) {
    celt_assert2!(fin.as_ptr() != fout.as_ptr(), "In-place FFT not supported");
    /* Bit-reverse the input */
    for i in 0..st.nfft as usize {
        fout[st.bitrev[i] as usize] = fin[i];
    }
    for i in 0..st.nfft as usize {
        fout[i].i = -fout[i].i;
    }
    opus_fft_impl(st, fout);
    for i in 0..st.nfft as usize {
        fout[i].i = -fout[i].i;
    }
}

/// `opus_ifft()` — the C macro `((void)(arch), opus_ifft_c(...))`.
#[inline(always)]
pub fn opus_ifft(
    cfg: &kiss_fft_state,
    fin: &[kiss_fft_cpx],
    fout: &mut [kiss_fft_cpx],
    arch: i32,
) {
    let _ = arch;
    opus_ifft_c(cfg, fin, fout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The factor pairs `kf_factor` produces, stopping at the first `m == 1`.
    fn factor_pairs(st: &kiss_fft_state) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for i in 0..MAXFACTORS {
            let p = st.factors[2 * i] as i32;
            let m = st.factors[2 * i + 1] as i32;
            v.push((p, m));
            if m == 1 {
                break;
            }
        }
        v
    }

    #[test]
    fn kf_factor_sizes() {
        /* (n, the reversed factor list the C kf_factor() produces) */
        let cases: &[(i32, &[i32])] = &[
            (60, &[5, 3, 4]),
            (120, &[5, 3, 2, 4]),
            (240, &[5, 3, 4, 4]),
            (480, &[5, 3, 4, 2, 4]),
            (960, &[5, 3, 4, 4, 4]),
            (1920, &[5, 3, 4, 4, 2, 4]),
            (4, &[4]),
            (2, &[2]),
            (15, &[5, 3]),
            (6, &[3, 2]),
            (1, &[1]),
        ];
        for &(n, expect) in cases {
            let mut facbuf = [0i16; 2 * MAXFACTORS];
            assert_eq!(kf_factor(n, &mut facbuf), 1, "kf_factor({n}) failed");
            let got: Vec<i32> = (0..expect.len()).map(|i| facbuf[2 * i] as i32).collect();
            assert_eq!(got, expect, "kf_factor({n})");
        }
        /* Primes larger than 5 are rejected. */
        for n in [7, 11, 49, 77] {
            let mut facbuf = [0i16; 2 * MAXFACTORS];
            assert_eq!(kf_factor(n, &mut facbuf), 0, "kf_factor({n})");
        }
    }

    #[test]
    fn kf_factor_m_progression() {
        for n in [60, 120, 240, 480, 960, 1920, 3840] {
            let st = opus_fft_alloc(n, 0).expect("fft alloc");
            let pairs = factor_pairs(&st);
            let mut prod = 1i32;
            for &(p, m) in &pairs {
                assert!((1..=5).contains(&p), "radix {p} for nfft {n}");
                prod *= p;
                /* facbuf[2*i+1] is "stage's fft length / p", i.e. the product of
                   the remaining radices *including* the current one. */
                assert_eq!(m, n / prod, "m mismatch for nfft {n}");
            }
            assert_eq!(prod, n);
        }
    }

    #[test]
    fn fft_round_trip() {
        let n = 120;
        let st = opus_fft_alloc(n, 0).expect("fft alloc");
        let mut x = vec![kiss_fft_cpx::default(); n as usize];
        let mut s: u32 = 0x1234_5678;
        let mut next = || {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            (s as i32) as f32 / 131072.0
        };
        for v in x.iter_mut() {
            v.r = next();
            v.i = next();
        }
        let mut f = vec![kiss_fft_cpx::default(); n as usize];
        let mut y = vec![kiss_fft_cpx::default(); n as usize];
        opus_fft_c(&st, &x, &mut f);
        opus_ifft_c(&st, &f, &mut y);
        /* opus_fft_c scales by 1/nfft and opus_ifft_c does not scale, so the
           round trip is the identity. */
        let mut maxerr = 0.0f32;
        let mut maxabs = 0.0f32;
        for i in 0..n as usize {
            maxerr = maxerr.max((y[i].r - x[i].r).abs());
            maxerr = maxerr.max((y[i].i - x[i].i).abs());
            maxabs = maxabs.max(x[i].r.abs()).max(x[i].i.abs());
        }
        assert!(maxerr < 1e-5 * maxabs, "round-trip error {maxerr}");
    }

    #[test]
    fn fft_matches_naive_dft() {
        /* Sanity check of the sign/scaling conventions against a direct DFT. */
        let n = 30;
        let st = opus_fft_alloc(n, 0).expect("fft alloc");
        let mut x = vec![kiss_fft_cpx::default(); n as usize];
        let mut s: u32 = 0xdead_beef;
        for v in x.iter_mut() {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            v.r = (s as i32) as f32 / 131072.0;
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            v.i = (s as i32) as f32 / 131072.0;
        }
        let mut f = vec![kiss_fft_cpx::default(); n as usize];
        opus_fft_c(&st, &x, &mut f);
        for k in 0..n as usize {
            let mut re = 0.0f64;
            let mut im = 0.0f64;
            for (t, xt) in x.iter().enumerate() {
                let ph = -2.0 * core::f64::consts::PI * (k * t) as f64 / n as f64;
                re += xt.r as f64 * ph.cos();
                im += xt.r as f64 * ph.sin();
                re += -(xt.i as f64) * ph.sin();
                im += xt.i as f64 * ph.cos();
            }
            let (re, im) = ((re / n as f64) as f32, (im / n as f64) as f32);
            assert!((f[k].r - re).abs() < 1e-3, "re {k}: {} vs {re}", f[k].r);
            assert!((f[k].i - im).abs() < 1e-3, "im {k}: {} vs {im}", f[k].i);
        }
    }

    #[test]
    fn shared_twiddles_and_bitrev() {
        let base = opus_fft_alloc(480, 0).expect("base");
        let sub = opus_fft_alloc_twiddles(120, Some(&base), 0).expect("sub");
        assert_eq!(base.shift, -1);
        assert_eq!(sub.shift, 2);
        assert_eq!(sub.scale, 1.0f32 / 120.0);
        /* The shared table is the base's, as in C where the pointer is copied. */
        assert_eq!(sub.twiddles.len(), base.twiddles.len());
        assert_eq!(sub.twiddles, base.twiddles);
        /* bitrev is a permutation of 0..nfft */
        let mut seen = vec![false; 120];
        for &b in &sub.bitrev {
            assert!(!seen[b as usize]);
            seen[b as usize] = true;
        }
        assert!(seen.into_iter().all(|x| x));
    }
}

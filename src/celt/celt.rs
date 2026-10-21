/* Copyright (c) 2007-2008 CSIRO
   Copyright (c) 2007-2010 Xiph.Org Foundation
   Copyright (c) 2008 Gregory Maxwell
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

//! Transliteration of `celt/celt.h` + `celt/celt.c` (libopus 1.6.1) into safe
//! Rust.  This is a derived work of the BSD-3-Clause libopus sources; the
//! copyright notice above is the one of the original files.
//!
//! Configuration (see `docs/porting-conventions.md`): floating point,
//! `CUSTOM_MODES` defined, `ENABLE_HARDENING` defined, x86 intrinsics + RTCD
//! enabled.  `ENABLE_QEXT` is **undefined**, so:
//!
//! * `comb_filter_qext()` (and the `overlap == 240` re-dispatch in
//!   `comb_filter()`) is not compiled;
//! * `resampling_factor()` has no 96 kHz case;
//! * the `#ifdef ENABLE_QEXT` table half of `celt.h` is not present.
//!
//! `FIXED_POINT` is undefined, so every `SATURATE`, `SHL32`, `MULT_COEF*`
//! macro is the float-build identity/`*`; they are still written out as the
//! `crate::celt::arch` functions so the code can be diffed against the C.
//!
//! The reference library is built with `OPUS_X86_PRESUME_SSE` defined, so the
//! `comb_filter_const` macro in `celt/pitch.h` resolves to
//! `comb_filter_const_sse` (`celt/x86/pitch_sse.c`) and **not** to the plain-C
//! `comb_filter_const_c`.  [`comb_filter`] therefore calls
//! [`crate::celt::kernels::comb_filter_const`] (the verified emulation of that
//! SSE kernel) and never [`comb_filter_const_c`].  For the same reason the
//! reference library does not export `comb_filter_const_c` at all (the
//! definition in `celt.c` is `#if`'d out and `NON_STATIC_COMB_FILTER_CONST_C`
//! is not defined); the crate has exactly one implementation of it, in
//! `kernels.rs`, re-exported below under the `celt.c` name.
//!
//! Functions declared by `celt.h` but implemented in other `celt/*.c` files
//! are deliberately **not** duplicated here:
//!
//! | declaration | definition |
//! | --- | --- |
//! | `celt_preemphasis()` | `celt/celt_encoder.c` |
//! | `deemphasis()` | `celt/celt_decoder.c` |
//! | `celt_synthesis()` | `celt/celt_decoder.c` |
//! | `celt_{encoder,decoder}_get_size/init()` | `celt/celt_{encoder,decoder}.c` |
//! | `celt_encode_with_ec()`, `celt_decode_with_ec*()` | `celt/celt_{encoder,decoder}.c` |
//! | `celt_encoder_ctl` / `celt_decoder_ctl` (`= opus_custom_*_ctl`) | `celt/celt_{encoder,decoder}.c` |
//! | `opus_get_version_string()` | already ported in `src/opus/mod.rs` |
//!
//! `celt.h` has no enums.  Its struct typedefs are all accounted for:
//! [`AnalysisInfo`] and [`SILKInfo`] are defined below, `CELTMode` is already
//! `OpusCustomMode` in `src/celt/modes.rs`, and the opaque `CELTEncoder` /
//! `CELTDecoder` are `#define`s to `OpusCustomEncoder` / `OpusCustomDecoder`,
//! whose struct bodies live in `celt/celt_encoder.c` and `celt/celt_decoder.c`
//! respectively (neither file is ported yet).

use super::arch::*;
use super::kernels;
use super::modes::CELTMode;
use crate::types::*;

/* ------------------------------------------------------------------------- *
 * celt/celt.h: constants
 * ------------------------------------------------------------------------- */

/// `LEAK_BANDS` (`celt/celt.h`).
pub const LEAK_BANDS: i32 = 19;

/// `QEXT_EXTENSION_ID` (`celt/celt.h`).  `ENABLE_QEXT` is undefined, so nothing
/// in the reference configuration uses it; the `#define` itself is unguarded
/// and is kept for completeness.
pub const QEXT_EXTENSION_ID: i32 = 124;

/// `COMBFILTER_MAXPERIOD` (`celt/celt.h`).
pub const COMBFILTER_MAXPERIOD: i32 = 1024;
/// `COMBFILTER_MINPERIOD` (`celt/celt.h`).
pub const COMBFILTER_MINPERIOD: i32 = 15;

/* ------------------------------------------------------------------------- *
 * Compile-time bounds replacing the C `VARDECL`/`ALLOC` sizes.
 *
 * The C declares its per-frame temporaries with `VARDECL`/`ALLOC`, which are
 * `alloca`-style *stack* allocations sized by run-time mode/frame values, so a
 * C frame allocates nothing on the heap.  Rust has no `alloca`, so the port
 * uses fixed-size stack arrays; the constants below are the proven maxima of
 * the C run-time sizes and are used only as array lengths (the code still
 * slices them down to the exact C size, so no arithmetic changes).
 * ------------------------------------------------------------------------- */

/// Upper bound on the CELT frame size `N = mode->shortMdctSize << LM`.
///
/// Without `ENABLE_QEXT`, `opus_custom_mode_create()` rejects
/// `frame_size > 1024`, and `celt_encode_with_ec()` only accepts the frame
/// sizes `mode->shortMdctSize << LM` with `LM <= mode->maxLM`, i.e. at most
/// the frame size the mode was created with.
pub const MAX_CELT_FRAME: usize = 1024;

/// Upper bound on `mode->overlap` (`= (shortMdctSize>>2)<<2`, `celt/modes.c`):
/// the mode creator requires `shortMdctSize*300 <= Fs` and `Fs <= 96000`, so
/// `overlap <= shortMdctSize <= 320`.
pub const MAX_CELT_OVERLAP: usize = 320;

/// Upper bound on `mode->nbEBands`.  `compute_ebands()` (`celt/modes.c`)
/// returns `low+high` where `high = nBark-lin <= BARK_BANDS-lin` and, because
/// `bark_freq[lin] < lin*res` for the first `lin` with a spacing `>= res`,
/// `low <= lin`; hence `nbEBands <= BARK_BANDS == 25`.
pub const MAX_NB_EBANDS: usize = 25;

/// `MAX_CHANNELS` (`celt/celt.h`) — every CELT allocation here is `C`/`CC`
/// wide.
pub const MAX_CHANNELS: usize = 2;

/// Upper bound on one PVQ band, `(eBands[nbEBands]-eBands[nbEBands-1])<<LM`.
/// Without `SMALL_FOOTPRINT`, `opus_custom_mode_create()` rejects any mode
/// whose largest band (`<< maxLM`) exceeds 208.
pub const MAX_PVQ_N: usize = 208;

/* Encoder/decoder requests (`celt/celt.h`).  The C code spells the
   function-like macros (`CELT_SET_PREDICTION(x)`, ...) as a request id
   followed by a checked argument; only the ids are meaningful in Rust. */
pub const CELT_SET_PREDICTION_REQUEST: i32 = 10002;
pub const CELT_SET_INPUT_CLIPPING_REQUEST: i32 = 10004;
pub const CELT_GET_AND_CLEAR_ERROR_REQUEST: i32 = 10007;
pub const CELT_SET_CHANNELS_REQUEST: i32 = 10008;
pub const CELT_SET_START_BAND_REQUEST: i32 = 10010;
pub const CELT_SET_END_BAND_REQUEST: i32 = 10012;
pub const CELT_GET_MODE_REQUEST: i32 = 10015;
pub const CELT_SET_SIGNALLING_REQUEST: i32 = 10016;
pub const CELT_SET_TONALITY_REQUEST: i32 = 10018;
pub const CELT_SET_TONALITY_SLOPE_REQUEST: i32 = 10020;
pub const CELT_SET_ANALYSIS_REQUEST: i32 = 10022;
pub const OPUS_SET_LFE_REQUEST: i32 = 10024;
pub const OPUS_SET_ENERGY_MASK_REQUEST: i32 = 10026;
pub const CELT_SET_SILK_INFO_REQUEST: i32 = 10028;

/* ------------------------------------------------------------------------- *
 * celt/celt.h: structs
 * ------------------------------------------------------------------------- */

/// `AnalysisInfo` (`celt/celt.h`).
///
/// C leaves it to `celt_encoder.c` to zero it; the `Default` impl here matches
/// that "all bytes zero" initialisation.
#[derive(Clone, Copy, Default)]
pub struct AnalysisInfo {
    pub valid: i32,
    pub tonality: f32,
    pub tonality_slope: f32,
    pub noisiness: f32,
    pub activity: f32,
    pub music_prob: f32,
    pub music_prob_min: f32,
    pub music_prob_max: f32,
    pub bandwidth: i32,
    pub activity_probability: f32,
    pub max_pitch_ratio: f32,
    /// `unsigned char leak_boost[LEAK_BANDS]` — stored as Q6.
    pub leak_boost: [u8; LEAK_BANDS as usize],
}

/// `SILKInfo` (`celt/celt.h`).
#[derive(Clone, Copy, Default)]
pub struct SILKInfo {
    pub signalType: i32,
    pub offset: i32,
}

/* ------------------------------------------------------------------------- *
 * celt/celt.h: static tables and inline helpers
 * ------------------------------------------------------------------------- */

/// `trim_icdf` (`celt/celt.h`).
pub static trim_icdf: [u8; 11] = [126, 124, 119, 109, 87, 41, 19, 9, 4, 2, 0];
/// `spread_icdf` (`celt/celt.h`).
pub static spread_icdf: [u8; 4] = [25, 23, 2, 0];
/// `tapset_icdf` (`celt/celt.h`).
pub static tapset_icdf: [u8; 3] = [2, 1, 0];

/// `toOpusTable` (`celt/celt.h`, `CUSTOM_MODES` path is the active one).
pub static toOpusTable: [u8; 20] = [
    0xE0, 0xE8, 0xF0, 0xF8, 0xC0, 0xC8, 0xD0, 0xD8, 0xA0, 0xA8, 0xB0, 0xB8, 0x00, 0x00, 0x00, 0x00,
    0x80, 0x88, 0x90, 0x98,
];

/// `fromOpusTable` (`celt/celt.h`, `CUSTOM_MODES` path is the active one).
pub static fromOpusTable: [u8; 16] = [
    0x80, 0x88, 0x90, 0x98, 0x40, 0x48, 0x50, 0x58, 0x20, 0x28, 0x30, 0x38, 0x00, 0x08, 0x10, 0x18,
];

/// `toOpus()` (`celt/celt.h`).
pub fn toOpus(c: u8) -> i32 {
    let mut ret: i32 = 0;
    if c < 0xA0 {
        ret = toOpusTable[(c >> 3) as usize] as i32;
    }
    if ret == 0 {
        -1
    } else {
        ret | ((c & 0x7) as i32)
    }
}

/// `fromOpus()` (`celt/celt.h`).
pub fn fromOpus(c: u8) -> i32 {
    if c < 0x80 {
        -1
    } else {
        (fromOpusTable[((c >> 3) - 16) as usize] as i32) | ((c & 0x7) as i32)
    }
}

/// `bits_to_bitrate()` (`static OPUS_INLINE` in `celt/celt.h`).
#[inline(always)]
pub fn bits_to_bitrate(bits: opus_int32, Fs: opus_int32, frame_size: opus_int32) -> opus_int32 {
    bits.wrapping_mul(6 * Fs / frame_size) / 6
}

/// `bitrate_to_bits()` (`static OPUS_INLINE` in `celt/celt.h`).
#[inline(always)]
pub fn bitrate_to_bits(bitrate: opus_int32, Fs: opus_int32, frame_size: opus_int32) -> opus_int32 {
    bitrate.wrapping_mul(6) / (6 * Fs / frame_size)
}

/* ------------------------------------------------------------------------- *
 * celt/celt.c
 * ------------------------------------------------------------------------- */

/// `resampling_factor()` (`celt/celt.c:62`).
///
/// `#ifdef ENABLE_QEXT` is undefined, so 96 kHz does not appear and falls into
/// the `default` case.  `CUSTOM_MODES` **is** defined, so the `celt_assert(0)`
/// is compiled out and the default returns 0.
pub fn resampling_factor(rate: opus_int32) -> i32 {
    let ret: i32;
    match rate {
        48000 => ret = 1,
        24000 => ret = 2,
        16000 => ret = 3,
        12000 => ret = 4,
        8000 => ret = 6,
        _ => {
            /* `#ifndef CUSTOM_MODES`: celt_assert(0); (not compiled here). */
            ret = 0;
        }
    }
    ret
}

/// `comb_filter_const_c()` (`celt/celt.c:166`, the non-`OPUS_ARM_ASM` body).
///
/// In the reference configuration this function is **not** compiled: `celt.c`
/// guards it with `#if !defined(OVERRIDE_COMB_FILTER_CONST) ||
/// defined(NON_STATIC_COMB_FILTER_CONST_C)`, and on x86 with `PRESUME_SSE`
/// `OVERRIDE_COMB_FILTER_CONST` is set while `NON_STATIC_COMB_FILTER_CONST_C`
/// is not.  `nm` confirms the symbol is absent from `libopus.a`; the C probe
/// uses a verbatim local copy instead.
///
/// The crate already contains exactly one transliteration of this body, in
/// [`crate::celt::kernels::comb_filter_const_c`] (verified by the `kernels`
/// probe); it is re-exported here so that the `celt.c` name resolves from this
/// module too.  Its slice convention, inherited from the C pointer arithmetic,
/// is documented on the kernel: `x` must start `T+2` samples *before* the C
/// `x` pointer (`x_rust[i] == x_c[i - T - 2]`), with
/// `y.len() >= N` and `x.len() >= N + max(4, T + 2)`.
pub use super::kernels::comb_filter_const_c;

/// `comb_filter_const` **macro** (`celt/pitch.h:197`) as resolved in the
/// reference configuration: `PRESUME_SSE` and `!FIXED_POINT` make it
/// `comb_filter_const_sse(y, x, T, N, g10, g11, g12)` (the `arch` argument is
/// discarded by `((void)(arch), ...)`).
#[inline(always)]
fn comb_filter_const(y: &mut [f32], x: &[f32], T: i32, N: i32, g10: f32, g11: f32, g12: f32) {
    kernels::comb_filter_const(y, x, T, N, g10, g11, g12);
}

/// `OPUS_MOVE(dst, src, n)` (`celt/os_support.h`, `memmove`).
#[inline(always)]
fn opus_move(buf: &mut [f32], src: usize, dst: usize, n: usize) {
    buf.copy_within(src..src + n, dst);
}

/// The body of `comb_filter()`, factored over a single backing buffer so that
/// the C aliasing cases (`x != y` and `x == y`) can both be expressed safely.
///
/// `c_x[i] == buf[x_off + i]` and `c_y[i] == buf[y_off + i]` for all `i` the C
/// function touches.  When `x_off == y_off` the C in-place semantics apply: a
/// read of `x[i - T]` observes the value previously *written* to that position,
/// which is exactly what the C pointer aliasing does.
///
/// `ENABLE_QEXT` is undefined, so the `if (overlap == 240) comb_filter_qext(..)`
/// early-out of `celt.c` is not part of the reference configuration and is not
/// transliterated.
fn comb_filter_impl(
    buf: &mut [f32],
    x_off: usize,
    y_off: usize,
    T0: i32,
    T1: i32,
    N: i32,
    g0: f32,
    g1: f32,
    tapset0: i32,
    tapset1: i32,
    window: &[f32],
    overlap: i32,
    arch: i32,
) {
    let _ = arch;
    /* `static const opus_val16 gains[3][3]` (`celt.c:246`).  In the float build
       `QCONST16(x,bits)` is `(x)`, so the C`QCONST16(...)` argument is the
       literal value. */
    static GAINS: [[f32; 3]; 3] = [
        [0.3066406250, 0.2170410156, 0.1296386719],
        [0.4638671875, 0.2680664062, 0.],
        [0.7998046875, 0.1000976562, 0.],
    ];

    let mut i: i32;

    /* `#ifdef ENABLE_QEXT` `if (overlap == 240) { comb_filter_qext(...); return; }`
       is not compiled in the reference configuration. */

    if g0 == 0.0 && g1 == 0.0 {
        /* OPT: Happens to work without the OPUS_MOVE(), but only because the
           current encoder already copies x to y. */
        if x_off != y_off {
            opus_move(buf, x_off, y_off, N as usize);
        }
        return;
    }
    /* When the gain is zero, T0 and/or T1 is set to zero.  We need to have then
       be at least 2 to avoid processing garbage data. */
    let T0 = IMAX(T0, COMBFILTER_MINPERIOD);
    let T1 = IMAX(T1, COMBFILTER_MINPERIOD);
    let T0u = T0 as usize;
    let T1u = T1 as usize;
    let g00 = MULT_COEF_TAPS(g0, GAINS[tapset0 as usize][0]);
    let g01 = MULT_COEF_TAPS(g0, GAINS[tapset0 as usize][1]);
    let g02 = MULT_COEF_TAPS(g0, GAINS[tapset0 as usize][2]);
    let g10 = MULT_COEF_TAPS(g1, GAINS[tapset1 as usize][0]);
    let g11 = MULT_COEF_TAPS(g1, GAINS[tapset1 as usize][1]);
    let g12 = MULT_COEF_TAPS(g1, GAINS[tapset1 as usize][2]);
    let mut x1 = buf[x_off - T1u + 1];
    let mut x2 = buf[x_off - T1u];
    let mut x3 = buf[x_off - T1u - 1];
    let mut x4 = buf[x_off - T1u - 2];
    /* If the filter didn't change, we don't need the overlap. */
    let mut overlap = overlap;
    if g0 == g1 && T0 == T1 && tapset0 == tapset1 {
        overlap = 0;
    }
    i = 0;
    while i < overlap {
        let ii = i as usize;
        let x0 = buf[x_off + ii + 2 - T1u];
        let f = MULT_COEF(window[ii], window[ii]);
        buf[y_off + ii] = buf[x_off + ii]
            + MULT_COEF_32(
                MULT_COEF(COEF_ONE - f, g00),
                buf[x_off + ii - T0u],
            )
            + MULT_COEF_32(
                MULT_COEF(COEF_ONE - f, g01),
                ADD32(buf[x_off + ii + 1 - T0u], buf[x_off + ii - T0u - 1]),
            )
            + MULT_COEF_32(
                MULT_COEF(COEF_ONE - f, g02),
                ADD32(buf[x_off + ii + 2 - T0u], buf[x_off + ii - T0u - 2]),
            )
            + MULT_COEF_32(MULT_COEF(f, g10), x2)
            + MULT_COEF_32(MULT_COEF(f, g11), ADD32(x1, x3))
            + MULT_COEF_32(MULT_COEF(f, g12), ADD32(x0, x4));
        /* `y[i] = SATURATE(y[i], SIG_SAT)` is the identity in the float build. */
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
        i += 1;
    }
    if g1 == 0.0 {
        /* OPT: Happens to work without the OPUS_MOVE(), but only because the
           current encoder already copies x to y. */
        if x_off != y_off {
            let n = (N - overlap) as usize;
            opus_move(buf, x_off + overlap as usize, y_off + overlap as usize, n);
        }
        return;
    }

    /* Compute the part with the constant filter.
     *
     * The C calls `comb_filter_const(y+i, x+i, T1, N-i, ...)` once.  That
     * kernel (see `crate::celt::kernels::comb_filter_const`) is a pure function
     * of a *copy* of its source window, so calling it on the whole tail at once
     * would skip the feedback an in-place call produces: for a 4-sample output
     * block starting at `j` it reads `x[j-T1-2 .. j-T1+6]`, i.e. up to
     * `T1-6` samples behind the write pointer, which an aliased call has
     * already overwritten.
     *
     * The tail is therefore processed in chunks of 12 samples, copying the
     * window out of `buf` before each chunk.  For a chunk starting at `b` the
     * largest history read is `b+8-T1+6 = b-T1+14`, which for `T1 >= 15` is at
     * most `b-1`: no chunk ever reads a position it has itself written, so the
     * copy can be taken once up front and still holds exactly the values the C
     * in-place kernel would see (already-written feedback before `b`, original
     * samples at and after `b`).  The rolling `x0v` shuffle state is reproduced
     * exactly as well, because 12 is a multiple of 4 and each chunk therefore
     * starts on the same absolute 4-sample block boundary as the single call.
     */
    let rem = N - i;
    let base = i; /* `comb_filter_const(y+i, x+i, ...)`: the tail starts at `i` */
    if x_off + N as usize == y_off {
        /* `x != y` in C: the caller copied the source into `buf[..h+N]` and the
           outputs go to the disjoint region `buf[h+N..]`, so the C's single
           `comb_filter_const(y+i, x+i, T1, N-i, ...)` can be issued as one call
           over the whole tail.  Its source window
           `[x_off+base-T1-2, x_off+base+rem) = [.., x_off+N) = [.., y_off)`
           ends before the first output sample and therefore never observes a
           written value — exactly the C's read set.  (This is the shape the
           encoder's `run_prefilter` uses; the chunked path below exists only
           for the aliased in-place decoder case.) */
        let wstart = x_off + base as usize - T1u - 2;
        let wlen = (rem + T1 + 2) as usize;
        let ystart = y_off + base as usize;
        debug_assert!(wstart + wlen <= ystart, "comb_filter: tail window overlaps y");
        let (lo, hi) = buf.split_at_mut(ystart);
        comb_filter_const(
            &mut hi[..rem as usize],
            &lo[wstart..wstart + wlen],
            T1,
            rem,
            g10,
            g11,
            g12,
        );
    } else {
        let mut b: i32 = 0;
        /* The per-chunk window copy of the C's aliased read (`comb_filter_const`
           takes `&mut y` and `&x` over the same buffer, which Rust cannot express).
           The C has no allocation here at all; the size is the C bound
           `IMIN(12, rem) + T1 + 2 <= 12 + COMBFILTER_MAXPERIOD + 2`, so this is
           stack storage rather than a fresh `Vec` per chunk. */
        let mut w_storage = [0.0f32; 12 + COMBFILTER_MAXPERIOD as usize + 2];
        while b < rem {
            let m = IMIN(12, rem - b);
            let wstart = x_off + (base + b) as usize - T1u - 2;
            let wlen = (m + T1 + 2) as usize;
            let w = &mut w_storage[..wlen];
            w.copy_from_slice(&buf[wstart..wstart + wlen]);
            let ystart = y_off + (base + b) as usize;
            comb_filter_const(
                &mut buf[ystart..ystart + m as usize],
                &w,
                T1,
                m,
                g10,
                g11,
                g12,
            );
            b += m;
        }
    }
}

/// `comb_filter()` (`celt/celt.c:238`).
///
/// Rust cannot hand the same buffer to `&mut y` and `&x`, so the two C
/// aliasing cases get two entry points:
///
/// * this one takes distinct `y`/`x` slices (`x != y` in C);
/// * [`comb_filter_in_place`] takes one buffer and an offset (`x == y` in C).
///
/// Both run the same [`comb_filter_impl`], so the arithmetic and the read/write
/// order are identical to the C in each case.
///
/// # Slice convention
/// Negative C indices into `x` are expressed by shifting the slice:
/// `x[k] == x_c[k - H]`, where
/// `H = IMAX(IMAX(T0, T1), COMBFILTER_MINPERIOD) + 2`.  The caller must pass at
/// least `H + N` samples; `x[0]` is therefore the sample `H` positions before
/// the first output sample.  `y` is the C `y` (`y[0] == y_c[0]`, `N` samples)
/// and `window` must hold at least `overlap` samples.
pub fn comb_filter(
    y: &mut [f32],
    x: &[f32],
    T0: i32,
    T1: i32,
    N: i32,
    g0: f32,
    g1: f32,
    tapset0: i32,
    tapset1: i32,
    window: &[f32],
    overlap: i32,
    arch: i32,
) {
    let h = (IMAX(IMAX(T0, T1), COMBFILTER_MINPERIOD) + 2) as usize;
    let n = N as usize;
    debug_assert!(x.len() >= h + n, "comb_filter: x needs H + N samples");
    debug_assert!(y.len() >= n, "comb_filter: y needs N samples");
    debug_assert!(overlap >= 0 && overlap <= N);
    /* The x and y regions must be disjoint in the shared scratch buffer: in C
       the two buffers never overlap when `x != y`.
       `h = IMAX(IMAX(T0,T1),COMBFILTER_MINPERIOD)+2 <= COMBFILTER_MAXPERIOD+2`
       and `n = N <= MAX_CELT_FRAME`, so the C's ad-hoc `buf` (which the C does
       not allocate at all — it aliases the caller's arrays) is stack storage. */
    debug_assert!(h <= COMBFILTER_MAXPERIOD as usize + 2, "comb_filter: T0/T1 too large");
    debug_assert!(n <= MAX_CELT_FRAME, "comb_filter: N too large");
    let mut buf_storage = [0f32; COMBFILTER_MAXPERIOD as usize + 2 + 2 * MAX_CELT_FRAME];
    let mut buf = &mut buf_storage[..h + 2 * n];
    buf[..h + n].copy_from_slice(&x[..h + n]);
    comb_filter_impl(
        &mut buf, h, h + n, T0, T1, N, g0, g1, tapset0, tapset1, window, overlap, arch,
    );
    y[..n].copy_from_slice(&buf[h + n..h + 2 * n]);
}

/// In-place (`x == y`) form of [`comb_filter`].
///
/// `buf[x_off + i]` is `x_c[i] == y_c[i]` and everything before `x_off` is the
/// history (`buf[k] == x_c[k - x_off]`).  The C decoder calls
/// `comb_filter(out_syn[c], out_syn[c], ...)`, and that aliasing is
/// *observable*: the overlap loop and the constant tail read positions that
/// have already been overwritten.  The caller must pass
/// `x_off >= IMAX(IMAX(T0, T1), COMBFILTER_MINPERIOD) + 2` and
/// `buf.len() >= x_off + N`.
pub fn comb_filter_in_place(
    buf: &mut [f32],
    x_off: i32,
    T0: i32,
    T1: i32,
    N: i32,
    g0: f32,
    g1: f32,
    tapset0: i32,
    tapset1: i32,
    window: &[f32],
    overlap: i32,
    arch: i32,
) {
    let xo = x_off as usize;
    debug_assert!(x_off >= IMAX(IMAX(T0, T1), COMBFILTER_MINPERIOD) + 2);
    debug_assert!(buf.len() >= xo + N as usize);
    debug_assert!(overlap >= 0 && overlap <= N);
    comb_filter_impl(
        buf, xo, xo, T0, T1, N, g0, g1, tapset0, tapset1, window, overlap, arch,
    );
}

/// TF change table (`celt/celt.c:320`).  Positive values mean better frequency
/// resolution (longer effective window), negative values better time
/// resolution.  The second index is `4*isTransient + 2*tf_select +
/// per_band_flag`.
pub static tf_select_table: [[i8; 8]; 4] = [
    /* isTransient=0     isTransient=1 */
    [0, -1, 0, -1, 0, -1, 0, -1],  /* 2.5 ms */
    [0, -1, 0, -2, 1, 0, 1, -1],   /* 5 ms */
    [0, -2, 0, -3, 2, 0, 1, -1],   /* 10 ms */
    [0, -2, 0, -3, 3, 0, 1, -1],   /* 20 ms */
];

/// `init_caps()` (`celt/celt.c:329`).
///
/// `cap` must hold at least `m.nbEBands` entries.
pub fn init_caps(m: &CELTMode, cap: &mut [i32], LM: i32, C: i32) {
    for i in 0..m.nbEBands {
        let N = (m.eBands[(i + 1) as usize] as i32 - m.eBands[i as usize] as i32) << LM;
        cap[i as usize] = (m.cache.caps[(m.nbEBands * (2 * LM + C - 1) + i) as usize] as i32 + 64)
            * C
            * N
            >> 2;
    }
}

/// `opus_strerror()` (`celt/celt.c:342`).
pub fn opus_strerror(error: i32) -> &'static str {
    static ERROR_STRINGS: [&str; 8] = [
        "success",
        "invalid argument",
        "buffer too small",
        "internal error",
        "corrupted stream",
        "request not implemented",
        "invalid state",
        "memory allocation failed",
    ];
    if error > 0 || error < -7 {
        "unknown error"
    } else {
        ERROR_STRINGS[(-error) as usize]
    }
}

/* `opus_get_version_string()` is defined in `celt/celt.c` in libopus, but the
   port already has it in `src/opus/mod.rs`; it is not duplicated here. */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::modes::mode48000_960_120;

    #[test]
    fn resampling_factor_matches_c() {
        assert_eq!(resampling_factor(48000), 1);
        assert_eq!(resampling_factor(24000), 2);
        assert_eq!(resampling_factor(16000), 3);
        assert_eq!(resampling_factor(12000), 4);
        assert_eq!(resampling_factor(8000), 6);
        /* `ENABLE_QEXT` is undefined: 96 kHz is not special-cased. */
        assert_eq!(resampling_factor(96000), 0);
        /* `CUSTOM_MODES` is defined: the default returns 0 without asserting. */
        assert_eq!(resampling_factor(44100), 0);
        assert_eq!(resampling_factor(0), 0);
        assert_eq!(resampling_factor(-1), 0);
        assert_eq!(resampling_factor(7999), 0);
        assert_eq!(resampling_factor(48001), 0);
    }

    #[test]
    fn init_caps_matches_static_mode_tables() {
        let m = mode48000_960_120();
        assert_eq!(m.nbEBands, 21);
        let mut cap = [0i32; 21];

        init_caps(&m, &mut cap, 0, 1);
        assert_eq!(
            cap,
            [
                72, 72, 72, 72, 72, 72, 72, 72, 112, 112, 112, 112, 249, 249, 249, 363, 363, 464,
                594, 562, 555
            ]
        );

        init_caps(&m, &mut cap, 0, 2);
        assert_eq!(
            cap,
            [
                144, 144, 144, 144, 144, 144, 144, 144, 304, 304, 304, 304, 542, 542, 542, 786, 786,
                988, 1248, 1170, 1144
            ]
        );

        init_caps(&m, &mut cap, 1, 1);
        assert_eq!(
            cap,
            [
                112, 112, 112, 112, 112, 112, 112, 112, 249, 249, 249, 249, 514, 514, 514, 741, 741,
                944, 1212, 1152, 1122
            ]
        );

        init_caps(&m, &mut cap, 3, 2);
        assert_eq!(
            cap,
            [
                1072, 1072, 1072, 1072, 1072, 1072, 1072, 1072, 2120, 2120, 2120, 2120, 4192, 4192,
                4192, 6024, 6024, 7648, 9792, 9360, 9152
            ]
        );
    }

    #[test]
    fn opus_strerror_matches_c() {
        assert_eq!(opus_strerror(0), "success");
        assert_eq!(opus_strerror(-1), "invalid argument");
        assert_eq!(opus_strerror(-7), "memory allocation failed");
        assert_eq!(opus_strerror(1), "unknown error");
        assert_eq!(opus_strerror(-8), "unknown error");
    }
}

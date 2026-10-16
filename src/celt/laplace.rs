//! Mirror of `celt/laplace.{c,h}`.
/* Copyright (c) 2007 CSIRO
   Copyright (c) 2007-2009 Xiph.Org Foundation
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
//!
//! This file is a transliteration of `celt/laplace.c` and `celt/laplace.h`
//! (libopus 1.6.1, floating-point configuration) into safe Rust.  The C names,
//! the operation order and the constants are preserved verbatim; only the
//! pointer arguments became references.
//!
//! Verification: `compat/c/probe_laplace.c` and `examples/probe_laplace.rs` run
//! the identical deterministic sequence against the reference library and the
//! port; `bash compat/scripts/run-probe.sh laplace` must print `IDENTICAL`.

use super::arch::*;
use super::entcode::*;
use super::entdec::*;
use super::entenc::*;
use crate::types::*;

/* The minimum probability of an energy delta (out of 32768). */
/// `LAPLACE_LOG_MINP`
pub const LAPLACE_LOG_MINP: i32 = 0;
/// `LAPLACE_MINP`
pub const LAPLACE_MINP: u32 = 1u32 << LAPLACE_LOG_MINP;
/* The minimum number of guaranteed representable energy deltas (in one
    direction). */
/// `LAPLACE_NMIN`
pub const LAPLACE_NMIN: u32 = 16;

/// When called, decay is positive and at most 11456.
///
/// `static` in C; `pub` here so that the bit-exactness probe can call it
/// directly (it has no external symbol in the reference archive).
pub fn ec_laplace_get_freq1(fs0: u32, decay: i32) -> u32 {
    let ft: u32;
    ft = 32768u32
        .wrapping_sub(LAPLACE_MINP.wrapping_mul(2 * LAPLACE_NMIN))
        .wrapping_sub(fs0);
    ft.wrapping_mul((16384 - decay) as u32) >> 15
}

/// `ec_laplace_encode()`
pub fn ec_laplace_encode(enc: &mut ec_ctx, value: &mut i32, mut fs: u32, decay: i32) {
    let mut fl: u32;
    let mut val: i32 = *value;
    fl = 0;
    if val != 0 {
        let s: i32;
        let mut i: i32;
        s = -((val < 0) as i32);
        val = (val.wrapping_add(s)) ^ s;
        fl = fs;
        fs = ec_laplace_get_freq1(fs, decay);
        /* Search the decaying part of the PDF.*/
        i = 1;
        while fs > 0 && i < val {
            fs = fs.wrapping_mul(2);
            fl = fl.wrapping_add(fs.wrapping_add(2 * LAPLACE_MINP));
            fs = (fs.wrapping_mul(decay as u32)) >> 15;
            i += 1;
        }
        /* Everything beyond that has probability LAPLACE_MINP. */
        if fs == 0 {
            let mut di: i32;
            let mut ndi_max: i32;
            ndi_max = ((32768u32
                .wrapping_sub(fl)
                .wrapping_add(LAPLACE_MINP)
                .wrapping_sub(1))
                >> LAPLACE_LOG_MINP) as i32;
            ndi_max = (ndi_max - s) >> 1;
            di = IMIN(val - i, ndi_max - 1);
            fl = fl.wrapping_add(((2 * di + 1 + s) as u32).wrapping_mul(LAPLACE_MINP));
            fs = IMIN(LAPLACE_MINP, 32768u32.wrapping_sub(fl));
            *value = (i + di + s) ^ s;
        } else {
            fs = fs.wrapping_add(LAPLACE_MINP);
            fl = fl.wrapping_add(fs & ((!s) as u32));
        }
        celt_assert!(fl.wrapping_add(fs) <= 32768);
        celt_assert!(fs > 0);
    }
    ec_encode_bin(enc, fl, fl.wrapping_add(fs), 15);
}

/// `ec_laplace_decode()`
pub fn ec_laplace_decode(dec: &mut ec_ctx, mut fs: u32, decay: i32) -> i32 {
    let mut val: i32 = 0;
    let mut fl: u32;
    let mut fm: u32;
    fm = ec_decode_bin(dec, 15);
    fl = 0;
    if fm >= fs {
        val += 1;
        fl = fs;
        fs = ec_laplace_get_freq1(fs, decay).wrapping_add(LAPLACE_MINP);
        /* Search the decaying part of the PDF.*/
        while fs > LAPLACE_MINP && fm >= fl.wrapping_add(2 * fs) {
            fs = fs.wrapping_mul(2);
            fl = fl.wrapping_add(fs);
            fs = fs
                .wrapping_sub(2 * LAPLACE_MINP)
                .wrapping_mul(decay as u32)
                >> 15;
            fs = fs.wrapping_add(LAPLACE_MINP);
            val += 1;
        }
        /* Everything beyond that has probability LAPLACE_MINP. */
        if fs <= LAPLACE_MINP {
            let mut di: i32;
            di = (fm.wrapping_sub(fl) >> (LAPLACE_LOG_MINP + 1)) as i32;
            val += di;
            fl = fl.wrapping_add(((2 * di) as u32).wrapping_mul(LAPLACE_MINP));
        }
        if fm < fl.wrapping_add(fs) {
            val = -val;
        } else {
            fl = fl.wrapping_add(fs);
        }
    }
    celt_assert!(fl < 32768);
    celt_assert!(fs > 0);
    celt_assert!(fl <= fm);
    celt_assert!(fm < IMIN(fl.wrapping_add(fs), 32768));
    ec_dec_update(dec, fl, IMIN(fl.wrapping_add(fs), 32768), 32768);
    val
}

/// `ec_laplace_encode_p0()`
pub fn ec_laplace_encode_p0(enc: &mut ec_ctx, value: i32, p0: opus_uint16, decay: opus_uint16) {
    let s: i32;
    let mut sign_icdf = [0u16; 3];
    sign_icdf[0] = (32768 - p0 as i32) as u16;
    sign_icdf[1] = sign_icdf[0] / 2;
    sign_icdf[2] = 0;
    s = if value == 0 {
        0
    } else if value > 0 {
        1
    } else {
        2
    };
    ec_enc_icdf16(enc, s, &sign_icdf, 15);
    let mut value: i32 = value.abs();
    if value != 0 {
        let mut icdf = [0u16; 8];
        icdf[0] = IMAX(7, decay);
        for i in 1i32..7 {
            icdf[i as usize] = IMAX(
                7 - i,
                (icdf[i as usize - 1] as i32).wrapping_mul(decay as i32) >> 15,
            ) as u16;
        }
        icdf[7] = 0;
        value -= 1;
        loop {
            ec_enc_icdf16(enc, IMIN(value, 7), &icdf, 15);
            value -= 7;
            if !(value >= 0) {
                break;
            }
        }
    }
}

/// `ec_laplace_decode_p0()`
pub fn ec_laplace_decode_p0(dec: &mut ec_ctx, p0: opus_uint16, decay: opus_uint16) -> i32 {
    let mut s: i32;
    let mut sign_icdf = [0u16; 3];
    sign_icdf[0] = (32768 - p0 as i32) as u16;
    sign_icdf[1] = sign_icdf[0] / 2;
    sign_icdf[2] = 0;
    s = ec_dec_icdf16(dec, &sign_icdf, 15);
    if s == 2 {
        s = -1;
    }
    if s != 0 {
        let mut value: i32;
        let mut icdf = [0u16; 8];
        icdf[0] = IMAX(7, decay);
        for i in 1i32..7 {
            icdf[i as usize] = IMAX(
                7 - i,
                (icdf[i as usize - 1] as i32).wrapping_mul(decay as i32) >> 15,
            ) as u16;
        }
        icdf[7] = 0;
        value = 1;
        let mut v: i32;
        loop {
            v = ec_dec_icdf16(dec, &icdf, 15);
            value += v;
            if v != 7 {
                break;
            }
        }
        s * value
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start-frequency helper used by `celt/tests/test_unit_laplace.c`.
    fn ec_laplace_get_start_freq(decay: i32) -> u32 {
        let ft: u32 = 32768 - LAPLACE_MINP * (2 * LAPLACE_NMIN + 1);
        let fs: i32 = (ft.wrapping_mul((16384 - decay) as u32) / (16384 + decay) as u32) as i32;
        (fs as u32).wrapping_add(LAPLACE_MINP)
    }

    #[test]
    fn freq1_matches_closed_form() {
        /* With fs0 == 0, ft == 32768 - LAPLACE_MINP*2*LAPLACE_NMIN == 32736,
           so ec_laplace_get_freq1() is the plain Q15 scaling of the `decay`
           complement. */
        for decay in [0, 1, 100, 5000, 11456, 16383] {
            assert_eq!(
                ec_laplace_get_freq1(0, decay),
                (32736u32.wrapping_mul((16384 - decay) as u32)) >> 15
            );
        }
    }

    #[test]
    fn freq1_is_monotone_and_bounded() {
        for decay in [0, 1000, 6000, 11456] {
            let mut prev = u32::MAX;
            for fs0 in [0u32, 1, 2, 31, 32, 100, 1000, 20000, 32700] {
                let f = ec_laplace_get_freq1(fs0, decay);
                assert!(f <= prev, "freq1 not non-increasing at fs0={fs0}");
                assert!(f < 32768);
                prev = f;
            }
            /* fs0 == 32736 leaves ft == 0. */
            assert_eq!(ec_laplace_get_freq1(32736, decay), 0);
        }
    }

    #[test]
    fn freq1_reconstructs_start_freq() {
        /* ec_laplace_get_start_freq() is the value of fs such that the decode
           loop's first step matches the encoder's `fs += LAPLACE_MINP`. */
        for decay in [5000, 6000, 8000, 11000] {
            let fs0 = ec_laplace_get_start_freq(decay);
            assert!(fs0 > LAPLACE_MINP);
            assert!(ec_laplace_get_freq1(fs0, decay) < fs0);
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut state: u32 = 0x9E3779B9;
        let mut vals = [0i32; 300];
        let mut decays = [0i32; 300];
        for i in 0..vals.len() {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            vals[i] = (state % 15) as i32 - 7;
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            decays[i] = (state % 11000) as i32 + 5000;
        }
        let fs_of = |i: usize| ec_laplace_get_start_freq(decays[i]);

        let mut enc = ec_enc_init(40000);
        for i in 0..vals.len() {
            ec_laplace_encode(&mut enc, &mut vals[i], fs_of(i), decays[i]);
        }
        ec_enc_done(&mut enc);

        let buf = enc.ec_get_buffer()[..enc.ec_range_bytes() as usize].to_vec();
        let mut dec = ec_dec_init(&buf);
        for i in 0..vals.len() {
            assert_eq!(ec_laplace_decode(&mut dec, fs_of(i), decays[i]), vals[i]);
        }
    }

    #[test]
    fn encode_decode_p0_round_trip() {
        let mut state: u32 = 0x12345678;
        let mut vals = [0i32; 200];
        let mut p0s = [0u16; 200];
        let mut decays = [0u16; 200];
        for i in 0..vals.len() {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            vals[i] = (state % 41) as i32 - 20;
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            p0s[i] = (state % 32000 + 1) as u16;
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            decays[i] = (state % 16000 + 1) as u16;
        }

        let mut enc = ec_enc_init(40000);
        for i in 0..vals.len() {
            ec_laplace_encode_p0(&mut enc, vals[i], p0s[i], decays[i]);
        }
        ec_enc_done(&mut enc);

        let buf = enc.ec_get_buffer()[..enc.ec_range_bytes() as usize].to_vec();
        let mut dec = ec_dec_init(&buf);
        for i in 0..vals.len() {
            assert_eq!(ec_laplace_decode_p0(&mut dec, p0s[i], decays[i]), vals[i]);
        }
    }
}

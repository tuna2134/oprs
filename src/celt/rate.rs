/* Copyright (c) 2007-2008 CSIRO
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

//! Transliteration of `celt/rate.h` and `celt/rate.c` (libopus 1.6.1) into safe
//! Rust.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! Configuration: `CUSTOM_MODES` **is** defined, so `fits_in32()` and
//! `compute_pulse_cache()` are part of the build; `ENABLE_QEXT` and `FUZZING`
//! are **not**, so `clt_compute_extra_allocation()`, `ec_enc_depth()` /
//! `ec_dec_depth()` and the `FUZZING` alternative of the skip decision are
//! dropped.  `USE_SMALL_DIV_TABLE` is not defined on x86-64, so `celt_udiv()`
//! is a plain division (see `crate::celt::entcode`).
//!
//! The C `const int *` arrays (`offsets`, `cap`, `bits1`, `bits2`, `thresh`)
//! and the `int *` in/out arrays (`pulses`, `ebits`, `fine_priority`) become
//! slices; `int *intensity`, `int *dual_stereo` and `opus_int32 *_balance`
//! become `&mut`.  Every arithmetic that C performs modulo 2^32 or that can
//! overflow a signed 32-bit integer is written with the explicit
//! `wrapping_*` form.

use super::arch::*;
use super::celt::MAX_NB_EBANDS;
use super::cwrs::get_required_bits;
use super::entcode::*;
use super::entdec::{ec_dec_bit_logp, ec_dec_uint};
use super::entenc::{ec_enc_bit_logp, ec_enc_uint};
use super::modes::*;
use crate::types::*;

/// `MAX_PSEUDO` (celt/rate.h)
pub const MAX_PSEUDO: i32 = 40;
/// `LOG_MAX_PSEUDO` (celt/rate.h)
pub const LOG_MAX_PSEUDO: i32 = 6;
/// `CELT_MAX_PULSES` (celt/rate.h)
pub const CELT_MAX_PULSES: i32 = 128;
/// `MAX_FINE_BITS` (celt/rate.h)
pub const MAX_FINE_BITS: i32 = 8;
/// `FINE_OFFSET` (celt/rate.h)
pub const FINE_OFFSET: i32 = 21;
/// `QTHETA_OFFSET` (celt/rate.h)
pub const QTHETA_OFFSET: i32 = 4;
/// `QTHETA_OFFSET_TWOPHASE` (celt/rate.h)
pub const QTHETA_OFFSET_TWOPHASE: i32 = 16;
/// `ALLOC_STEPS` (celt/rate.c)
const ALLOC_STEPS: i32 = 6;

/// `LOG2_FRAC_TABLE[24]` (celt/rate.c)
// python3 compat/scripts/gen-tables.py reference/opus-1.6.1/celt/rate.c LOG2_FRAC_TABLE
pub static LOG2_FRAC_TABLE: [u8; 24] = [
    0, 8, 13, 16, 19, 21, 23, 24, 26, 27, 28, 29, 30, 31, 32, 32, 33, 34,
    34, 35, 36, 36, 37, 37,
];

/// `get_pulses()` (celt/rate.h)
#[inline(always)]
pub fn get_pulses(i: i32) -> i32 {
    if i < 8 { i } else { (8 + (i & 7)) << ((i >> 3) - 1) }
}

/// `bits2pulses()` (celt/rate.h)
#[inline(always)]
pub fn bits2pulses(m: &CELTMode, band: i32, LM: i32, bits: i32) -> i32 {
    let LM = LM + 1;
    let off = m.cache.index[(LM * m.nbEBands + band) as usize] as i32;
    let cache = &m.cache.bits[off as usize..];

    let mut lo = 0i32;
    let mut hi = cache[0] as i32;
    let bits = bits - 1;
    for _ in 0..LOG_MAX_PSEUDO {
        let mid = (lo + hi + 1) >> 1;
        /* OPT: Make sure this is implemented with a conditional move. */
        if cache[mid as usize] as i32 >= bits {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    if bits - (if lo == 0 { -1 } else { cache[lo as usize] as i32 })
        <= cache[hi as usize] as i32 - bits
    {
        lo
    } else {
        hi
    }
}

/// `pulses2bits()` (celt/rate.h)
#[inline(always)]
pub fn pulses2bits(m: &CELTMode, band: i32, LM: i32, pulses: i32) -> i32 {
    let LM = LM + 1;
    let off = m.cache.index[(LM * m.nbEBands + band) as usize] as i32;
    let cache = &m.cache.bits[off as usize..];
    if pulses == 0 {
        0
    } else {
        cache[pulses as usize] as i32 + 1
    }
}

/// `fits_in32()` (`static` in celt/rate.c): determines if V(N,K) fits in a
/// 32-bit unsigned integer.  N and K are themselves limited to 15 bits.
fn fits_in32(_n: i32, _k: i32) -> bool {
    static maxN: [opus_int16; 15] = [
        32767, 32767, 32767, 1476, 283, 109, 60, 40, 29, 24, 20, 18, 16, 14, 13,
    ];
    static maxK: [opus_int16; 15] = [
        32767, 32767, 32767, 32767, 1172, 238, 95, 53, 36, 27, 22, 18, 16, 15, 13,
    ];
    if _n >= 14 {
        if _k >= 14 {
            false
        } else {
            _n <= maxN[_k as usize] as i32
        }
    } else {
        _k <= maxK[_n as usize] as i32
    }
}

/// `compute_pulse_cache()` (celt/rate.c).  `CUSTOM_MODES` is defined here.
pub fn compute_pulse_cache(m: &mut CELTMode, LM: i32) {
    let mut curr: i32 = 0;
    let mut nbEntries: i32 = 0;
    /* C uses `int entryN[100]` / `entryK[100]` / `entryI[100]`; the vectors
       grow if a mode ever needs more than 100 unique band sizes (the reference
       would overflow the stack arrays there). */
    let mut entryN = vec![0i32; 100];
    let mut entryK = vec![0i32; 100];
    let mut entryI = vec![0i32; 100];
    let mut cindex = vec![0i16; (m.nbEBands * (LM + 2)) as usize];

    /* Scan for all unique band sizes. */
    for i in 0..=LM + 1 {
        for j in 0..m.nbEBands {
            let N = ((m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32) << i) >> 1;
            cindex[(i * m.nbEBands + j) as usize] = -1;
            /* Find other bands that have the same size. */
            for k in 0..=i {
                let mut n = 0i32;
                while n < m.nbEBands && (k != i || n < j) {
                    if N == ((m.eBands[(n + 1) as usize] as i32 - m.eBands[n as usize] as i32) << k) >> 1
                    {
                        cindex[(i * m.nbEBands + j) as usize] =
                            cindex[(k * m.nbEBands + n) as usize];
                        break;
                    }
                    n += 1;
                }
            }
            if cindex[(i * m.nbEBands + j) as usize] == -1 && N != 0 {
                /* C uses `int entryN[100]` and would overflow for a mode with
                   more than 100 unique band sizes (UB in the reference); the
                   guard keeps the port panic-free without changing any
                   well-defined case. */
                if nbEntries as usize >= entryN.len() {
                    let n = nbEntries as usize + 1;
                    entryN.resize(n, 0);
                    entryK.resize(n, 0);
                    entryI.resize(n, 0);
                }
                entryN[nbEntries as usize] = N;
                let mut K = 0i32;
                while fits_in32(N, get_pulses(K + 1)) && K < MAX_PSEUDO {
                    K += 1;
                }
                entryK[nbEntries as usize] = K;
                cindex[(i * m.nbEBands + j) as usize] = curr as i16;
                entryI[nbEntries as usize] = curr;

                curr += K + 1;
                nbEntries += 1;
            }
        }
    }
    let mut bits = vec![0u8; curr as usize];
    m.cache.size = curr;
    /* Compute the cache for all unique sizes. */
    for i in 0..nbEntries {
        let entryI_i = entryI[i as usize];
        let mut tmp = [0i16; (CELT_MAX_PULSES + 1) as usize];
        get_required_bits(
            &mut tmp,
            entryN[i as usize],
            get_pulses(entryK[i as usize]),
            BITRES,
        );
        for j in 1..=entryK[i as usize] {
            bits[(entryI_i + j) as usize] = (tmp[get_pulses(j) as usize] - 1) as u8;
        }
        bits[entryI_i as usize] = entryK[i as usize] as u8;
    }

    m.cache.bits = bits;

    /* Compute the maximum rate for each band at which we'll reliably use as
       many bits as we ask for. */
    let mut caps = vec![0u8; ((LM + 1) * 2 * m.nbEBands) as usize];
    let mut capidx = 0usize;
    for i in 0..=LM {
        for C in 1..=2 {
            for j in 0..m.nbEBands {
                let N0_0 = m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32;
                let mut max_bits: i32;
                /* N=1 bands only have a sign bit and fine bits. */
                if (N0_0 << i) == 1 {
                    max_bits = C * (1 + MAX_FINE_BITS) << BITRES;
                } else {
                    let mut N0 = N0_0;
                    let mut LM0 = 0i32;
                    /* Even-sized bands bigger than N=2 can be split one more time.
                       As of commit 44203907 all bands >1 are even, including custom
                       modes. */
                    if N0 > 2 {
                        N0 >>= 1;
                        LM0 -= 1;
                    }
                    /* N0=1 bands can't be split down to N<2. */
                    else if N0 <= 1 {
                        LM0 = IMIN(i, 1);
                        N0 <<= LM0;
                    }
                    /* Compute the cost for the lowest-level PVQ of a fully split
                       band. */
                    let off = cindex[((LM0 + 1) * m.nbEBands + j) as usize];
                    celt_assert!(off >= 0);
                    let pcache = &m.cache.bits[off as usize..];
                    max_bits = pcache[pcache[0] as usize] as i32 + 1;
                    /* Add in the cost of coding regular splits. */
                    let mut N = N0;
                    for k in 0..i - LM0 {
                        max_bits <<= 1;
                        /* Offset the number of qtheta bits by log2(N)/2
                           + QTHETA_OFFSET compared to their "fair share" of
                           total/N. */
                        let offset = ((m.logN[j as usize] as i32
                            + ((((LM0 + k) as opus_uint32) << BITRES) as i32))
                            >> 1)
                            - QTHETA_OFFSET;
                        /* The number of qtheta bits we'll allocate if the remainder
                           is to be max_bits.
                           The average measured cost for theta is 0.89701 times qb,
                           approximated here as 459/512. */
                        let num = 459 * ((2 * N - 1) * offset + max_bits);
                        let den = ((2 * N - 1) << 9) - 459;
                        let qb = IMIN((num + (den >> 1)) / den, 57);
                        celt_assert!(qb >= 0);
                        max_bits += qb;
                        N <<= 1;
                    }
                    /* Add in the cost of a stereo split, if necessary. */
                    if C == 2 {
                        max_bits <<= 1;
                        let offset = ((m.logN[j as usize] as i32 + (i << BITRES)) >> 1)
                            - (if N == 2 {
                                QTHETA_OFFSET_TWOPHASE
                            } else {
                                QTHETA_OFFSET
                            });
                        let ndof = 2 * N - 1 - (if N == 2 { 1 } else { 0 });
                        /* The average measured cost for theta with the step PDF is
                           0.95164 times qb, approximated here as 487/512. */
                        let num = (if N == 2 { 512 } else { 487 }) * (max_bits + ndof * offset);
                        let den = (ndof << 9) - (if N == 2 { 512 } else { 487 });
                        let qb = IMIN((num + (den >> 1)) / den, if N == 2 { 64 } else { 61 });
                        celt_assert!(qb >= 0);
                        max_bits += qb;
                    }
                    /* Add the fine bits we'll use. */
                    /* Compensate for the extra DoF in stereo. */
                    let ndof = C * N + (if C == 2 && N > 2 { 1 } else { 0 });
                    /* Offset the number of fine bits by log2(N)/2 + FINE_OFFSET
                       compared to their "fair share" of total/N. */
                    let mut offset =
                        ((m.logN[j as usize] as i32 + (i << BITRES)) >> 1) - FINE_OFFSET;
                    /* N=2 is the only point that doesn't match the curve. */
                    if N == 2 {
                        offset += (1 << BITRES) >> 2;
                    }
                    /* The number of fine bits we'll allocate if the remainder is
                       to be max_bits. */
                    let num = max_bits + ndof * offset;
                    let den = (ndof - 1) << BITRES;
                    let qb = IMIN((num + (den >> 1)) / den, MAX_FINE_BITS);
                    celt_assert!(qb >= 0);
                    max_bits += C * qb << BITRES;
                }
                max_bits = (4 * max_bits / (C * (N0_0 << i))) - 64;
                celt_assert!(max_bits >= 0);
                celt_assert!(max_bits < 256);
                caps[capidx] = max_bits as u8;
                capidx += 1;
            }
        }
    }
    m.cache.caps = caps;
    m.cache.index = cindex;
}

/// `interp_bits2pulses()` (`static OPUS_INLINE` in celt/rate.c).
pub fn interp_bits2pulses(
    m: &CELTMode,
    start: i32,
    end: i32,
    skip_start: i32,
    bits1: &[i32],
    bits2: &[i32],
    thresh: &[i32],
    cap: &[i32],
    mut total: opus_int32,
    _balance: &mut opus_int32,
    skip_rsv: i32,
    intensity: &mut i32,
    mut intensity_rsv: i32,
    dual_stereo: &mut i32,
    mut dual_stereo_rsv: i32,
    bits: &mut [i32],
    ebits: &mut [i32],
    fine_priority: &mut [i32],
    C: i32,
    LM: i32,
    ec: &mut ec_ctx,
    encode: i32,
    prev: i32,
    signalBandwidth: i32,
) -> i32 {
    let mut psum: opus_int32;
    let mut lo: i32;
    let mut hi: i32;
    let mut j: i32;
    let mut codedBands: i32 = -1;

    let alloc_floor: i32 = C << BITRES;
    let stereo: i32 = (C > 1) as i32;

    let logM: i32 = LM << BITRES;
    lo = 0;
    hi = 1 << ALLOC_STEPS;
    for _i in 0..ALLOC_STEPS {
        let mid = (lo + hi) >> 1;
        psum = 0;
        let mut done = 0i32;
        for j in (start..end).rev() {
            let tmp = bits1[j as usize] + ((mid * bits2[j as usize]) >> ALLOC_STEPS);
            if tmp >= thresh[j as usize] || done != 0 {
                done = 1;
                /* Don't allocate more than we can actually use. */
                psum += IMIN(tmp, cap[j as usize]);
            } else if tmp >= alloc_floor {
                psum += alloc_floor;
            }
        }
        if psum > total {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    psum = 0;
    /*printf ("interp bisection gave %d\n", lo);*/
    let mut done = 0i32;
    for j in (start..end).rev() {
        let mut tmp = bits1[j as usize] + ((lo * bits2[j as usize]) >> ALLOC_STEPS);
        if tmp < thresh[j as usize] && done == 0 {
            if tmp >= alloc_floor {
                tmp = alloc_floor;
            } else {
                tmp = 0;
            }
        } else {
            done = 1;
        }
        /* Don't allocate more than we can actually use. */
        tmp = IMIN(tmp, cap[j as usize]);
        bits[j as usize] = tmp;
        psum += tmp;
    }

    /* Decide which bands to skip, working backwards from the end. */
    codedBands = end;
    loop {
        let band_width: i32;
        let mut band_bits: i32;
        let rem: i32;
        j = codedBands - 1;
        /* Never skip the first band, nor a band that has been boosted by
           dynalloc.
           In the first case, we'd be coding a bit to signal we're going to waste
           all the other bits.
           In the second case, we'd be coding a bit to redistribute all the bits
           we just signaled should be concentrated in this band. */
        if j <= skip_start {
            /* Give the bit we reserved to end skipping back. */
            total += skip_rsv;
            break;
        }
        /* Figure out how many left-over bits we would be adding to this band.
           This can include bits we've stolen back from higher, skipped bands. */
        let mut left = total - psum;
        let mut percoeff = celt_udiv(
            left as opus_uint32,
            (m.eBands[codedBands as usize] as i32 - m.eBands[start as usize] as i32) as opus_uint32,
        ) as opus_int32;
        left = left.wrapping_sub(
            (m.eBands[codedBands as usize] as i32 - m.eBands[start as usize] as i32)
                .wrapping_mul(percoeff),
        );
        rem = IMAX(
            left.wrapping_sub(m.eBands[j as usize] as i32 - m.eBands[start as usize] as i32),
            0,
        );
        band_width = m.eBands[codedBands as usize] as i32 - m.eBands[j as usize] as i32;
        band_bits = bits[j as usize]
            .wrapping_add(percoeff.wrapping_mul(band_width))
            .wrapping_add(rem);
        /* Only code a skip decision if we're above the threshold for this band.
           Otherwise it is force-skipped.
           This ensures that we have enough bits to code the skip flag. */
        if band_bits >= IMAX(thresh[j as usize], alloc_floor + (1 << BITRES)) {
            if encode != 0 {
                /* This if() block is the only part of the allocation function that
                   is not a mandatory part of the bitstream: any bands we choose to
                   skip here must be explicitly signaled. */
                let depth_threshold: i32;
                /* We choose a threshold with some hysteresis to keep bands from
                   fluctuating in and out, but we try not to fold below a certain
                   point. */
                if codedBands > 17 {
                    depth_threshold = if j < prev { 7 } else { 9 };
                } else {
                    depth_threshold = 0;
                }
                /* `FUZZING` is not defined. */
                if codedBands <= start + 2
                    || (band_bits > (depth_threshold * band_width << LM << BITRES) >> 4
                        && j <= signalBandwidth)
                {
                    ec_enc_bit_logp(ec, 1, 1);
                    break;
                }
                ec_enc_bit_logp(ec, 0, 1);
            } else if ec_dec_bit_logp(ec, 1) != 0 {
                break;
            }
            /* We used a bit to skip this band. */
            psum += 1 << BITRES;
            band_bits -= 1 << BITRES;
        }
        /* Reclaim the bits originally allocated to this band. */
        psum -= bits[j as usize] + intensity_rsv;
        if intensity_rsv > 0 {
            intensity_rsv = LOG2_FRAC_TABLE[(j - start) as usize] as i32;
        }
        psum += intensity_rsv;
        if band_bits >= alloc_floor {
            /* If we have enough for a fine energy bit per channel, use it. */
            psum += alloc_floor;
            bits[j as usize] = alloc_floor;
        } else {
            /* Otherwise this band gets nothing at all. */
            bits[j as usize] = 0;
        }
        codedBands -= 1;
    }

    celt_assert!(codedBands > start);
    /* Code the intensity and dual stereo parameters. */
    if intensity_rsv > 0 {
        if encode != 0 {
            *intensity = IMIN(*intensity, codedBands);
            ec_enc_uint(
                ec,
                (*intensity - start) as opus_uint32,
                (codedBands + 1 - start) as opus_uint32,
            );
        } else {
            *intensity = start + ec_dec_uint(ec, (codedBands + 1 - start) as opus_uint32) as i32;
        }
    } else {
        *intensity = 0;
    }
    if *intensity <= start {
        total += dual_stereo_rsv;
        dual_stereo_rsv = 0;
    }
    if dual_stereo_rsv > 0 {
        if encode != 0 {
            ec_enc_bit_logp(ec, *dual_stereo, 1);
        } else {
            *dual_stereo = ec_dec_bit_logp(ec, 1);
        }
    } else {
        *dual_stereo = 0;
    }

    /* Allocate the remaining bits. */
    let mut left = total - psum;
    let den = m.eBands[codedBands as usize] as i32 - m.eBands[start as usize] as i32;
    let percoeff = celt_udiv(left as opus_uint32, den as opus_uint32) as opus_int32;
    left = left.wrapping_sub(den.wrapping_mul(percoeff));
    for j in start..codedBands {
        bits[j as usize] = bits[j as usize].wrapping_add(
            percoeff.wrapping_mul(m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32),
        );
    }
    for j in start..codedBands {
        let tmp = IMIN(
            left,
            m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32,
        );
        bits[j as usize] = bits[j as usize].wrapping_add(tmp);
        left = left.wrapping_sub(tmp);
    }

    let mut balance: opus_int32 = 0;
    j = start;
    while j < codedBands {
        let den: i32;
        let mut offset: i32;
        let NClogN: i32;
        let mut excess: opus_int32;

        celt_assert!(bits[j as usize] >= 0);
        let N0 = m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32;
        let N = N0 << LM;
        let bit = bits[j as usize] as opus_int32 + balance;

        if N > 1 {
            excess = MAX32(bit - cap[j as usize], 0);
            bits[j as usize] = bit - excess;

            /* Compensate for the extra DoF in stereo. */
            den = C * N
                + (if C == 2 && N > 2 && *dual_stereo == 0 && j < *intensity {
                    1
                } else {
                    0
                });

            NClogN = den * (m.logN[j as usize] as i32 + logM);

            /* Offset for the number of fine bits by log2(N)/2 + FINE_OFFSET
               compared to their "fair share" of total/N. */
            offset = (NClogN >> 1) - den * FINE_OFFSET;

            /* N=2 is the only point that doesn't match the curve. */
            if N == 2 {
                offset += (den << BITRES) >> 2;
            }

            /* Changing the offset for allocating the second and third
               fine energy bit. */
            if bits[j as usize] + offset < (den * 2) << BITRES {
                offset += NClogN >> 2;
            } else if bits[j as usize] + offset < (den * 3) << BITRES {
                offset += NClogN >> 3;
            }

            /* Divide with rounding. */
            ebits[j as usize] = IMAX(0, bits[j as usize] + offset + (den << (BITRES - 1)));
            ebits[j as usize] =
                (celt_udiv(ebits[j as usize] as opus_uint32, den as opus_uint32) >> BITRES) as i32;

            /* Make sure not to bust. */
            if C * ebits[j as usize] > (bits[j as usize] >> BITRES) {
                ebits[j as usize] = bits[j as usize] >> stereo >> BITRES;
            }

            /* More than that is useless because that's about as far as PVQ can go. */
            ebits[j as usize] = IMIN(ebits[j as usize], MAX_FINE_BITS);

            /* If we rounded down or capped this band, make it a candidate for the
               final fine energy pass. */
            fine_priority[j as usize] =
                (ebits[j as usize] * (den << BITRES) >= bits[j as usize] + offset) as i32;

            /* Remove the allocated fine bits; the rest are assigned to PVQ. */
            bits[j as usize] -= (C * ebits[j as usize]) << BITRES;
        } else {
            /* For N=1, all bits go to fine energy except for a single sign bit. */
            excess = MAX32(0, bit - (C << BITRES));
            bits[j as usize] = bit - excess;
            ebits[j as usize] = 0;
            fine_priority[j as usize] = 1;
        }

        /* Fine energy can't take advantage of the re-balancing in
           quant_all_bands().
           Instead, do the re-balancing here. */
        if excess > 0 {
            let extra_fine =
                IMIN(excess >> (stereo + BITRES), MAX_FINE_BITS - ebits[j as usize]);
            ebits[j as usize] += extra_fine;
            let extra_bits = (extra_fine * C) << BITRES;
            fine_priority[j as usize] = (extra_bits >= excess - balance) as i32;
            excess -= extra_bits;
        }
        balance = excess;

        celt_assert!(bits[j as usize] >= 0);
        celt_assert!(ebits[j as usize] >= 0);
        j += 1;
    }
    /* Save any remaining bits over the cap for the rebalancing in
       quant_all_bands(). */
    *_balance = balance;

    /* The skipped bands use all their bits for fine energy. */
    while j < end {
        ebits[j as usize] = bits[j as usize] >> stereo >> BITRES;
        celt_assert!((C * ebits[j as usize]) << BITRES == bits[j as usize]);
        bits[j as usize] = 0;
        fine_priority[j as usize] = (ebits[j as usize] < 1) as i32;
        j += 1;
    }
    codedBands
}

/// `clt_compute_allocation()` (celt/rate.c).
pub fn clt_compute_allocation(
    m: &CELTMode,
    start: i32,
    end: i32,
    offsets: &[i32],
    cap: &[i32],
    alloc_trim: i32,
    intensity: &mut i32,
    dual_stereo: &mut i32,
    total: opus_int32,
    balance: &mut opus_int32,
    pulses: &mut [i32],
    ebits: &mut [i32],
    fine_priority: &mut [i32],
    C: i32,
    LM: i32,
    ec: &mut ec_ctx,
    encode: i32,
    prev: i32,
    signalBandwidth: i32,
) -> i32 {
    let mut total = IMAX(total, 0);
    let len = m.nbEBands;
    let mut skip_start = start;
    /* Reserve a bit to signal the end of manually skipped bands. */
    let skip_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
    total -= skip_rsv;
    /* Reserve bits for the intensity and dual stereo parameters. */
    let mut intensity_rsv = 0;
    let mut dual_stereo_rsv = 0;
    if C == 2 {
        intensity_rsv = LOG2_FRAC_TABLE[(end - start) as usize] as i32;
        if intensity_rsv > total {
            intensity_rsv = 0;
        } else {
            total -= intensity_rsv;
            dual_stereo_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
            total -= dual_stereo_rsv;
        }
    }
    /* `VARDECL(int, bits1/bits2/thresh/trim_offset); ALLOC(..., len, int)` —
       `len` is `m.nbEBands <= MAX_NB_EBANDS`, so these are stack arrays. */
    let mut bits1_storage = [0i32; MAX_NB_EBANDS];
    let mut bits1 = &mut bits1_storage[..len as usize];
    let mut bits2_storage = [0i32; MAX_NB_EBANDS];
    let mut bits2 = &mut bits2_storage[..len as usize];
    let mut thresh_storage = [0i32; MAX_NB_EBANDS];
    let mut thresh = &mut thresh_storage[..len as usize];
    let mut trim_offset_storage = [0i32; MAX_NB_EBANDS];
    let mut trim_offset = &mut trim_offset_storage[..len as usize];

    for j in start..end {
        /* Below this threshold, we're sure not to allocate any PVQ bits. */
        thresh[j as usize] = IMAX(
            C << BITRES,
            (3 * (m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32) << LM
                << BITRES)
                >> 4,
        );
        /* Tilt of the allocation curve. */
        trim_offset[j as usize] = C
            * (m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32)
            * (alloc_trim - 5 - LM)
            * (end - j - 1)
            * (1 << (LM + BITRES))
            >> 6;
        /* Giving less resolution to single-coefficient bands because they get
           more benefit from having one coarse value per coefficient. */
        if (m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32) << LM == 1 {
            trim_offset[j as usize] -= C << BITRES;
        }
    }
    let mut lo = 1;
    let mut hi = m.nbAllocVectors - 1;
    loop {
        let mut done = 0i32;
        let mut psum = 0i32;
        let mid = (lo + hi) >> 1;
        for j in (start..end).rev() {
            let N = m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32;
            let mut bitsj =
                (C * N * m.allocVectors[(mid * len + j) as usize] as i32) << LM >> 2;
            if bitsj > 0 {
                bitsj = IMAX(0, bitsj + trim_offset[j as usize]);
            }
            bitsj += offsets[j as usize];
            if bitsj >= thresh[j as usize] || done != 0 {
                done = 1;
                /* Don't allocate more than we can actually use. */
                psum += IMIN(bitsj, cap[j as usize]);
            } else if bitsj >= C << BITRES {
                psum += C << BITRES;
            }
        }
        if psum > total {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
        /*printf ("lo = %d, hi = %d\n", lo, hi);*/
        if lo > hi {
            break;
        }
    }
    hi = lo;
    lo -= 1;
    /*printf ("interp between %d and %d\n", lo, hi);*/
    for j in start..end {
        let N = m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32;
        let mut bits1j = (C * N * m.allocVectors[(lo * len + j) as usize] as i32) << LM >> 2;
        let mut bits2j = if hi >= m.nbAllocVectors {
            cap[j as usize]
        } else {
            (C * N * m.allocVectors[(hi * len + j) as usize] as i32) << LM >> 2
        };
        if bits1j > 0 {
            bits1j = IMAX(0, bits1j + trim_offset[j as usize]);
        }
        if bits2j > 0 {
            bits2j = IMAX(0, bits2j + trim_offset[j as usize]);
        }
        if lo > 0 {
            bits1j += offsets[j as usize];
        }
        bits2j += offsets[j as usize];
        if offsets[j as usize] > 0 {
            skip_start = j;
        }
        bits2j = IMAX(0, bits2j - bits1j);
        bits1[j as usize] = bits1j;
        bits2[j as usize] = bits2j;
    }
    interp_bits2pulses(
        m,
        start,
        end,
        skip_start,
        &bits1,
        &bits2,
        &thresh,
        cap,
        total,
        balance,
        skip_rsv,
        intensity,
        intensity_rsv,
        dual_stereo,
        dual_stereo_rsv,
        pulses,
        ebits,
        fine_priority,
        C,
        LM,
        ec,
        encode,
        prev,
        signalBandwidth,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entenc::ec_enc_init;
    use crate::celt::modes::{
        cache_bits50, cache_caps50, cache_index50, mode48000_960_120, PulseCache,
    };

    #[test]
    fn get_pulses_matches_the_reference_sequence() {
        #[rustfmt::skip]
        let expected: [i32; 41] = [
            0, 1, 2, 3, 4, 5, 6, 7,
            8, 9, 10, 11, 12, 13, 14, 15,
            16, 18, 20, 22, 24, 26, 28, 30,
            32, 36, 40, 44, 48, 52, 56, 60,
            64, 72, 80, 88, 96, 104, 112, 120,
            128,
        ];
        for (i, &want) in expected.iter().enumerate() {
            assert_eq!(get_pulses(i as i32), want, "get_pulses({i})");
        }
        /* Strictly increasing over the used range, which is what the cache
           layout relies on. */
        for i in 1..=MAX_PSEUDO {
            assert!(get_pulses(i) > get_pulses(i - 1));
        }
    }

    #[test]
    fn fits_in32_boundaries() {
        assert!(fits_in32(1, 32767));
        assert!(!fits_in32(1, 32768));
        assert!(fits_in32(3, 32767));
        assert!(!fits_in32(4, 32767));
        assert!(fits_in32(13, 15));
        assert!(!fits_in32(13, 16));
        assert!(fits_in32(14, 13));
        assert!(!fits_in32(14, 14));
        assert!(!fits_in32(15, 13));
    }

    fn synthetic_mode(bits: Vec<u8>) -> CELTMode {
        let mut m = CELTMode::default();
        m.nbEBands = 1;
        m.maxLM = 0;
        m.cache = PulseCache {
            size: bits.len() as i32,
            /* `bits2pulses`/`pulses2bits` index the row for `LM+1`, so a
               `maxLM == 0` mode needs `maxLM+2 == 2` rows. */
            index: vec![0, 0],
            bits,
            caps: vec![0],
        };
        m
    }

    #[test]
    fn bits2pulses_round_trips_a_monotone_cache() {
        let mut bits = vec![0u8; 9];
        bits[0] = 8;
        for k in 1..=8usize {
            bits[k] = (k as u8) * 7;
        }
        let m = synthetic_mode(bits);
        assert_eq!(pulses2bits(&m, 0, 0, 0), 0);
        for k in 1..=8 {
            let b = pulses2bits(&m, 0, 0, k);
            assert_eq!(b, m.cache.bits[k as usize] as i32 + 1);
            assert_eq!(bits2pulses(&m, 0, 0, b), k);
        }
        /* For every bit budget the result stays inside the cache row. */
        for b in 1..=(m.cache.bits[8] as i32 + 2) {
            let k = bits2pulses(&m, 0, 0, b);
            assert!((0..=m.cache.bits[0] as i32).contains(&k), "b={b} k={k}");
        }
    }

    #[test]
    fn compute_pulse_cache_reproduces_the_static_tables() {
        let mut m = mode48000_960_120();
        m.cache = PulseCache::default();
        let lm = m.maxLM;
        compute_pulse_cache(&mut m, lm);
        assert_eq!(m.cache.size, 392);
        assert_eq!(m.cache.index, cache_index50.to_vec());
        assert_eq!(m.cache.bits, cache_bits50.to_vec());
        assert_eq!(m.cache.caps, cache_caps50.to_vec());
    }

    #[test]
    fn compute_allocation_smoke() {
        let m = mode48000_960_120();
        let len = m.nbEBands as usize;
        let offsets = vec![0i32; len];
        let cap = vec![64i32; len];
        let mut pulses = vec![0i32; len];
        let mut ebits = vec![0i32; len];
        let mut fine_priority = vec![0i32; len];
        let mut intensity = m.effEBands;
        let mut dual_stereo = 1;
        let mut balance = 0;
        let mut ec = ec_enc_init(512);
        let coded = clt_compute_allocation(
            &m,
            0,
            m.effEBands,
            &offsets,
            &cap,
            5,
            &mut intensity,
            &mut dual_stereo,
            1000,
            &mut balance,
            &mut pulses,
            &mut ebits,
            &mut fine_priority,
            2,
            3,
            &mut ec,
            1,
            0,
            20,
        );
        assert!(coded > 0 && coded <= m.effEBands);
        assert!(pulses.iter().all(|&p| p >= 0));
        assert!(ebits.iter().all(|&e| (0..=MAX_FINE_BITS).contains(&e)));
        assert!(fine_priority.iter().all(|&f| f == 0 || f == 1));
    }
}

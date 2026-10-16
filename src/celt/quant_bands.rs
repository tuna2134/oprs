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

//! Transliteration of `celt/quant_bands.h` and `celt/quant_bands.c`
//! (libopus 1.6.1, floating-point configuration) into safe Rust.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original files.
//!
//! Configuration (see `docs/porting-conventions.md`): `FIXED_POINT`,
//! `FLOAT_APPROX`, `ENABLE_QEXT` and `ENABLE_RES24` are **undefined**, so the
//! float branches are the live ones and `eMeans`, `pred_coef`, `beta_coef` and
//! `beta_intra` are the `float` variants of the tables.  `quant_bands.h` also
//! declares `log2Amp()`, but no translation unit in 1.6.1 defines it, so there
//! is no symbol to port.
//!
//! The C `const int *` / `int *` arrays became slices and the nullable `int *`
//! parameters (`prev_quant` in `quant_fine_energy()`/`unquant_fine_energy()`
//! and `oldEBands` in `quant_energy_finalise()`/`unquant_energy_finalise()`)
//! became `Option<&…>`; the `do { … } while (++c < C);` channel loops are
//! written as `loop { … }` with the increment at the bottom so that the
//! arithmetic order is identical.
//!
//! Verification: `compat/c/probe_quant_bands.c` and
//! `examples/probe_quant_bands.rs` run the identical deterministic sequence
//! against the reference library and the port; `bash
//! compat/scripts/run-probe.sh quant_bands` must print `IDENTICAL`.

use super::arch::*;
use super::entcode::*;
use super::entdec::{ec_dec_bits, ec_dec_bit_logp, ec_dec_icdf};
use super::entenc::{ec_enc_bits, ec_enc_bit_logp, ec_enc_icdf};
use super::laplace::{ec_laplace_decode, ec_laplace_encode};
use super::mathops::celt_log2_db;
use super::modes::*;
use super::rate::MAX_FINE_BITS;
use crate::types::*;

/// `celt/arch.h`: `#define DB_SHIFT 24`
///
/// The shared `arch` module does not carry it yet, so it is repeated here
/// verbatim.  In the float configuration every consumer shifts by it through
/// an identity macro, so its value never reaches the arithmetic — it only has
/// to match the C for the source to stay diffable.
pub const DB_SHIFT: i32 = 24;

/// `celt/arch.h`: `#define MAXG(a,b) MAX32(a, b)`
#[inline(always)]
fn MAXG<T: PartialOrd>(a: T, b: T) -> T {
    if a > b { a } else { b }
}

/* Mean energy in each band quantized in Q4 and converted back to float */
/// `quant_bands.c`: `const opus_val16 eMeans[25]` (float build; the C object is
/// a global symbol, hence `pub` here).
pub static eMeans: [opus_val16; 25] = [
    6.437500f32, 6.250000f32, 5.750000f32, 5.312500f32, 5.062500f32,
    4.812500f32, 4.500000f32, 4.375000f32, 4.875000f32, 4.687500f32,
    4.562500f32, 4.437500f32, 4.875000f32, 4.625000f32, 4.312500f32,
    4.500000f32, 4.375000f32, 4.625000f32, 4.750000f32, 4.437500f32,
    3.750000f32, 3.750000f32, 3.750000f32, 3.750000f32, 3.750000f32,
];
/* prediction coefficients: 0.9, 0.8, 0.65, 0.5 */
static pred_coef: [opus_val16; 4] = [
    (29440f64 / 32768.) as opus_val16,
    (26112f64 / 32768.) as opus_val16,
    (21248f64 / 32768.) as opus_val16,
    (16384f64 / 32768.) as opus_val16,
];
static beta_coef: [opus_val16; 4] = [
    (30147f64 / 32768.) as opus_val16,
    (22282f64 / 32768.) as opus_val16,
    (12124f64 / 32768.) as opus_val16,
    (6554f64 / 32768.) as opus_val16,
];
static beta_intra: opus_val16 = (4915f64 / 32768.) as opus_val16;

/*Parameters of the Laplace-like probability models used for the coarse energy.
  There is one pair of parameters for each frame size, prediction type
   (inter/intra), and band number.
  The first number of each pair is the probability of 0, and the second is the
   decay rate, both in Q8 precision.*/
static e_prob_model: [[[u8; 42]; 2]; 4] = [
    /*120 sample frames.*/
    [
        /*Inter*/
        [
            72, 127, 65, 129, 66, 128, 65, 128, 64, 128, 62, 128, 64, 128,
            64, 128, 92, 78, 92, 79, 92, 78, 90, 79, 116, 41, 115, 40,
            114, 40, 132, 26, 132, 26, 145, 17, 161, 12, 176, 10, 177, 11,
        ],
        /*Intra*/
        [
            24, 179, 48, 138, 54, 135, 54, 132, 53, 134, 56, 133, 55, 132,
            55, 132, 61, 114, 70, 96, 74, 88, 75, 88, 87, 74, 89, 66,
            91, 67, 100, 59, 108, 50, 120, 40, 122, 37, 97, 43, 78, 50,
        ],
    ],
    /*240 sample frames.*/
    [
        /*Inter*/
        [
            83, 78, 84, 81, 88, 75, 86, 74, 87, 71, 90, 73, 93, 74,
            93, 74, 109, 40, 114, 36, 117, 34, 117, 34, 143, 17, 145, 18,
            146, 19, 162, 12, 165, 10, 178, 7, 189, 6, 190, 8, 177, 9,
        ],
        /*Intra*/
        [
            23, 178, 54, 115, 63, 102, 66, 98, 69, 99, 74, 89, 71, 91,
            73, 91, 78, 89, 86, 80, 92, 66, 93, 64, 102, 59, 103, 60,
            104, 60, 117, 52, 123, 44, 138, 35, 133, 31, 97, 38, 77, 45,
        ],
    ],
    /*480 sample frames.*/
    [
        /*Inter*/
        [
            61, 90, 93, 60, 105, 42, 107, 41, 110, 45, 116, 38, 113, 38,
            112, 38, 124, 26, 132, 27, 136, 19, 140, 20, 155, 14, 159, 16,
            158, 18, 170, 13, 177, 10, 187, 8, 192, 6, 175, 9, 159, 10,
        ],
        /*Intra*/
        [
            21, 178, 59, 110, 71, 86, 75, 85, 84, 83, 91, 66, 88, 73,
            87, 72, 92, 75, 98, 72, 105, 58, 107, 54, 115, 52, 114, 55,
            112, 56, 129, 51, 132, 40, 150, 33, 140, 29, 98, 35, 77, 42,
        ],
    ],
    /*960 sample frames.*/
    [
        /*Inter*/
        [
            42, 121, 96, 66, 108, 43, 111, 40, 117, 44, 123, 32, 120, 36,
            119, 33, 127, 33, 134, 34, 139, 21, 147, 23, 152, 20, 158, 25,
            154, 26, 166, 21, 173, 16, 184, 13, 184, 10, 150, 13, 139, 15,
        ],
        /*Intra*/
        [
            22, 178, 63, 114, 74, 82, 84, 83, 92, 82, 103, 62, 96, 72,
            96, 67, 101, 73, 107, 72, 113, 55, 118, 52, 125, 52, 118, 52,
            117, 55, 135, 49, 137, 39, 157, 32, 145, 29, 97, 33, 77, 40,
        ],
    ],
];

static small_energy_icdf: [u8; 3] = [2, 1, 0];

/// `quant_bands.c`: `static opus_val32 loss_distortion()`
fn loss_distortion(
    eBands: &[celt_glog],
    oldEBands: &[celt_glog],
    start: i32,
    end: i32,
    len: i32,
    C: i32,
) -> opus_val32 {
    let mut c: i32;
    let mut dist: opus_val32 = 0 as opus_val32;
    c = 0;
    loop {
        for i in start..end {
            let d: celt_glog = PSHR32(
                SUB32(eBands[(i + c * len) as usize], oldEBands[(i + c * len) as usize]),
                DB_SHIFT - 7,
            );
            dist = MAC16_16(dist, d, d);
        }
        c += 1;
        if c >= C {
            break;
        }
    }
    MIN32(200 as opus_val32, SHR32(dist, 14))
}

/// `quant_bands.c`: `static int quant_coarse_energy_impl()`
fn quant_coarse_energy_impl(
    m: &CELTMode,
    start: i32,
    end: i32,
    eBands: &[celt_glog],
    oldEBands: &mut [celt_glog],
    budget: opus_int32,
    tell_: opus_int32,
    prob_model: &[u8],
    error: &mut [celt_glog],
    enc: &mut ec_ctx,
    C: i32,
    LM: i32,
    intra: i32,
    max_decay: celt_glog,
    lfe: i32,
) -> i32 {
    let mut c: i32;
    let mut badness: i32 = 0;
    let mut prev: [opus_val32; 2] = [0 as opus_val32, 0 as opus_val32];
    let coef: opus_val16;
    let beta: opus_val16;
    let mut tell: opus_int32 = tell_;

    if tell.wrapping_add(3) <= budget {
        ec_enc_bit_logp(enc, intra, 3);
    }
    if intra != 0 {
        coef = 0 as opus_val16;
        beta = beta_intra;
    } else {
        beta = beta_coef[LM as usize];
        coef = pred_coef[LM as usize];
    }

    /* Encode at a fixed coarse resolution */
    for i in start..end {
        c = 0;
        loop {
            let bits_left: i32;
            let mut qi: i32;
            let qi0: i32;
            let q: opus_val32;
            let x: celt_glog;
            let f: opus_val32;
            let tmp: opus_val32;
            let oldE: celt_glog;
            let decay_bound: celt_glog;
            x = eBands[(i + c * m.nbEBands) as usize];
            oldE = MAXG(-GCONST(9.0f32), oldEBands[(i + c * m.nbEBands) as usize]);
            f = x - coef * oldE - prev[c as usize];
            /* Rounding to nearest integer here is really important! */
            qi = (0.5f32 + f).floor() as i32;
            decay_bound = MAXG(-GCONST(28.0f32), oldEBands[(i + c * m.nbEBands) as usize]) - max_decay;
            /* Prevent the energy from going down too quickly (e.g. for bands
               that have just one bin) */
            if qi < 0 && x < decay_bound {
                qi += SHR32(SUB32(decay_bound, x), DB_SHIFT) as i32;
                if qi > 0 {
                    qi = 0;
                }
            }
            qi0 = qi;
            /* If we don't have enough bits to encode all the energy, just assume
                something safe. */
            tell = enc.ec_tell();
            bits_left = budget
                .wrapping_sub(tell)
                .wrapping_sub(3i32.wrapping_mul(C).wrapping_mul(end - i));
            if i != start && bits_left < 30 {
                if bits_left < 24 {
                    qi = IMIN(1, qi);
                }
                if bits_left < 16 {
                    qi = IMAX(-1, qi);
                }
            }
            if lfe != 0 && i >= 2 {
                qi = IMIN(qi, 0);
            }
            if budget.wrapping_sub(tell) >= 15 {
                let pi: i32 = 2 * IMIN(i, 20);
                ec_laplace_encode(
                    enc,
                    &mut qi,
                    (prob_model[pi as usize] as opus_uint32) << 7,
                    (prob_model[(pi + 1) as usize] as i32) << 6,
                );
            } else if budget.wrapping_sub(tell) >= 2 {
                qi = IMAX(-1, IMIN(qi, 1));
                ec_enc_icdf(enc, 2 * qi ^ -((qi < 0) as i32), &small_energy_icdf, 2);
            } else if budget.wrapping_sub(tell) >= 1 {
                qi = IMIN(0, qi);
                ec_enc_bit_logp(enc, -qi, 1);
            } else {
                qi = -1;
            }
            error[(i + c * m.nbEBands) as usize] =
                f - SHL32(EXTEND32(qi as opus_val32), DB_SHIFT);
            badness += (qi0 - qi).abs();
            q = SHL32(EXTEND32(qi as opus_val32), DB_SHIFT);

            tmp = MULT16_32_Q15(coef, oldE) + prev[c as usize] + q;
            oldEBands[(i + c * m.nbEBands) as usize] = tmp;
            prev[c as usize] = prev[c as usize] + q - MULT16_32_Q15(beta, q);
            c += 1;
            if c >= C {
                break;
            }
        }
    }
    return if lfe != 0 { 0 } else { badness };
}

pub fn quant_coarse_energy(
    m: &CELTMode,
    start: i32,
    end: i32,
    effEnd: i32,
    eBands: &[celt_glog],
    oldEBands: &mut [celt_glog],
    budget: opus_uint32,
    error: &mut [celt_glog],
    enc: &mut ec_ctx,
    C: i32,
    LM: i32,
    nbAvailableBytes: i32,
    force_intra: i32,
    delayedIntra: &mut opus_val32,
    mut two_pass: i32,
    loss_rate: i32,
    lfe: i32,
) {
    let mut intra: i32;
    let mut max_decay: celt_glog;
    let n: usize;
    let mut oldEBands_intra: Vec<celt_glog>;
    let mut error_intra: Vec<celt_glog>;
    let enc_start_state: ec_ctx;
    let mut tell: opus_uint32;
    let mut badness1: i32 = 0;
    let intra_bias: opus_int32;
    let new_distortion: opus_val32;

    intra = if force_intra != 0
        || (two_pass == 0
            && *delayedIntra > (2 * C * (end - start)) as opus_val32
            && nbAvailableBytes > (end - start) * C)
    {
        1
    } else {
        0
    };
    intra_bias = ((budget as opus_val32) * *delayedIntra * loss_rate as opus_val32
        / (C * 512) as opus_val32) as opus_int32;
    new_distortion = loss_distortion(eBands, oldEBands, start, effEnd, m.nbEBands, C);

    tell = enc.ec_tell() as opus_uint32;
    if tell.wrapping_add(3) > budget {
        two_pass = 0;
        intra = 0;
    }

    max_decay = GCONST(16.0f32);
    if end - start > 10 {
        max_decay = MIN32(max_decay, 0.125f32 * nbAvailableBytes as opus_val32);
    }
    if lfe != 0 {
        max_decay = GCONST(3.0f32);
    }
    enc_start_state = enc.clone();

    n = (C * m.nbEBands) as usize;
    oldEBands_intra = vec![0 as celt_glog; n];
    error_intra = vec![0 as celt_glog; n];
    oldEBands_intra.copy_from_slice(&oldEBands[..n]);

    if two_pass != 0 || intra != 0 {
        badness1 = quant_coarse_energy_impl(
            m,
            start,
            end,
            eBands,
            &mut oldEBands_intra,
            budget as opus_int32,
            tell as opus_int32,
            &e_prob_model[LM as usize][1],
            &mut error_intra,
            enc,
            C,
            LM,
            1,
            max_decay,
            lfe,
        );
    }

    if intra == 0 {
        let tell_intra: opus_int32;
        let enc_intra_state: ec_ctx;
        let nstart_bytes: opus_uint32;
        let nintra_bytes: opus_uint32;
        let save_bytes: opus_uint32;
        let intra_bits: Vec<u8>;
        let badness2: i32;

        tell_intra = ec_tell_frac(enc) as opus_int32;

        enc_intra_state = enc.clone();

        nstart_bytes = enc_start_state.ec_range_bytes();
        nintra_bytes = enc_intra_state.ec_range_bytes();
        save_bytes = nintra_bytes.wrapping_sub(nstart_bytes);
        /* intra_buf = ec_get_buffer(&enc_intra_state) + nstart_bytes; */
        intra_bits = enc_intra_state.ec_get_buffer()
            [nstart_bytes as usize..nstart_bytes.wrapping_add(save_bytes) as usize]
            .to_vec();

        *enc = enc_start_state.clone();

        badness2 = quant_coarse_energy_impl(
            m,
            start,
            end,
            eBands,
            oldEBands,
            budget as opus_int32,
            tell as opus_int32,
            &e_prob_model[LM as usize][intra as usize],
            error,
            enc,
            C,
            LM,
            0,
            max_decay,
            lfe,
        );

        if two_pass != 0
            && (badness1 < badness2
                || (badness1 == badness2
                    && (ec_tell_frac(enc) as opus_int32).wrapping_add(intra_bias) > tell_intra))
        {
            /* C does `*enc = enc_intra_state;` and then
               `OPUS_COPY(intra_buf, intra_bits, nintra_bytes-nstart_bytes);`.
               The C `ec_ctx` aliases the caller's output buffer, so the inter
               pass above clobbered the intra bit-stream and it had to be saved
               by hand.  Here `ec_ctx` owns its buffer, so assigning the saved
               state also restores the intra bits and the copy is implicit. */
            *enc = enc_intra_state;
            debug_assert_eq!(
                &enc.ec_get_buffer()
                    [nstart_bytes as usize..nstart_bytes.wrapping_add(save_bytes) as usize],
                &intra_bits[..]
            );
            oldEBands[..n].copy_from_slice(&oldEBands_intra);
            error[..n].copy_from_slice(&error_intra);
            intra = 1;
        }
    } else {
        oldEBands[..n].copy_from_slice(&oldEBands_intra);
        error[..n].copy_from_slice(&error_intra);
    }

    if intra != 0 {
        *delayedIntra = new_distortion;
    } else {
        *delayedIntra = ADD32(
            MULT16_32_Q15(
                MULT16_16_Q15(pred_coef[LM as usize], pred_coef[LM as usize]),
                *delayedIntra,
            ),
            new_distortion,
        );
    }
}

pub fn quant_fine_energy(
    m: &CELTMode,
    start: i32,
    end: i32,
    oldEBands: &mut [celt_glog],
    error: &mut [celt_glog],
    prev_quant: Option<&[i32]>,
    extra_quant: &[i32],
    enc: &mut ec_ctx,
    C: i32,
) {
    let mut c: i32;
    /* Encode finer resolution */
    for i in start..end {
        let extra: opus_int16;
        let prev: opus_int16;
        extra = (1i32 << extra_quant[i as usize]) as opus_int16;
        if extra_quant[i as usize] <= 0 {
            continue;
        }
        if enc.ec_tell() + C * extra_quant[i as usize] > (enc.storage * 8) as opus_int32 {
            continue;
        }
        prev = match prev_quant {
            Some(p) => p[i as usize] as opus_int16,
            None => 0,
        };
        c = 0;
        loop {
            let mut q2: i32;
            /* Has to be without rounding */
            q2 = ((error[(i + c * m.nbEBands) as usize] * (1i32 << prev) as celt_glog
                + 0.5f32)
                * extra as celt_glog)
                .floor() as i32;
            if q2 > extra as i32 - 1 {
                q2 = extra as i32 - 1;
            }
            if q2 < 0 {
                q2 = 0;
            }
            ec_enc_bits(enc, q2 as opus_uint32, extra_quant[i as usize] as u32);
            let mut offset: celt_glog = (q2 as celt_glog + 0.5f32)
                * (1i32 << (14 - extra_quant[i as usize])) as celt_glog
                * (1.0f32 / 16384 as celt_glog)
                - 0.5f32;
            offset *= (1i32 << (14 - prev as i32)) as celt_glog * (1.0f32 / 16384 as celt_glog);
            oldEBands[(i + c * m.nbEBands) as usize] += offset;
            error[(i + c * m.nbEBands) as usize] -= offset;
            /*printf ("%f ", error[i] - offset);*/
            c += 1;
            if c >= C {
                break;
            }
        }
    }
}

pub fn quant_energy_finalise(
    m: &CELTMode,
    start: i32,
    end: i32,
    mut oldEBands: Option<&mut [celt_glog]>,
    error: &mut [celt_glog],
    fine_quant: &[i32],
    fine_priority: &[i32],
    bits_left_: i32,
    enc: &mut ec_ctx,
    C: i32,
) {
    let mut prio: i32;
    let mut c: i32;
    let mut bits_left: i32 = bits_left_;

    /* Use up the remaining bits */
    prio = 0;
    loop {
        let mut i: i32 = start;
        while i < end && bits_left >= C {
            if fine_quant[i as usize] >= MAX_FINE_BITS || fine_priority[i as usize] != prio {
                i += 1;
                continue;
            }
            c = 0;
            loop {
                let q2: i32;
                let offset: celt_glog;
                q2 = if error[(i + c * m.nbEBands) as usize] < 0.0f32 {
                    0
                } else {
                    1
                };
                ec_enc_bits(enc, q2 as opus_uint32, 1);
                offset = (q2 as celt_glog - 0.5f32)
                    * (1i32 << (14 - fine_quant[i as usize] - 1)) as celt_glog
                    * (1.0f32 / 16384 as celt_glog);
                if let Some(ref mut be) = oldEBands {
                    be[(i + c * m.nbEBands) as usize] += offset;
                }
                error[(i + c * m.nbEBands) as usize] -= offset;
                bits_left -= 1;
                c += 1;
                if c >= C {
                    break;
                }
            }
            i += 1;
        }
        prio += 1;
        if prio >= 2 {
            break;
        }
    }
}

pub fn unquant_coarse_energy(
    m: &CELTMode,
    start: i32,
    end: i32,
    oldEBands: &mut [celt_glog],
    intra: i32,
    dec: &mut ec_ctx,
    C: i32,
    LM: i32,
) {
    let prob_model: &[u8] = &e_prob_model[LM as usize][intra as usize];
    let mut c: i32;
    let mut prev: [opus_val64; 2] = [0 as opus_val64, 0 as opus_val64];
    let coef: opus_val16;
    let beta: opus_val16;
    let budget: opus_int32;
    let mut tell: opus_int32;

    if intra != 0 {
        coef = 0 as opus_val16;
        beta = beta_intra;
    } else {
        beta = beta_coef[LM as usize];
        coef = pred_coef[LM as usize];
    }

    budget = dec.storage.wrapping_mul(8) as opus_int32;

    /* Decode at a fixed coarse resolution */
    for i in start..end {
        c = 0;
        loop {
            let qi: i32;
            let q: opus_val32;
            let tmp: opus_val32;
            /* It would be better to express this invariant as a
               test on C at function entry, but that isn't enough
               to make the static analyzer happy. */
            celt_sig_assert!(c < 2);
            tell = dec.ec_tell();
            if budget.wrapping_sub(tell) >= 15 {
                let pi: i32 = 2 * IMIN(i, 20);
                qi = ec_laplace_decode(
                    dec,
                    (prob_model[pi as usize] as opus_uint32) << 7,
                    (prob_model[(pi + 1) as usize] as i32) << 6,
                );
            } else if budget.wrapping_sub(tell) >= 2 {
                let mut qi_icdf: i32 = ec_dec_icdf(dec, &small_energy_icdf, 2);
                qi_icdf = (qi_icdf >> 1) ^ -((qi_icdf & 1) as i32);
                qi = qi_icdf;
            } else if budget.wrapping_sub(tell) >= 1 {
                qi = -ec_dec_bit_logp(dec, 1);
            } else {
                qi = -1;
            }
            q = SHL32(EXTEND32(qi as opus_val32), DB_SHIFT);

            oldEBands[(i + c * m.nbEBands) as usize] =
                MAXG(-GCONST(9.0f32), oldEBands[(i + c * m.nbEBands) as usize]);
            tmp = MULT16_32_Q15(coef, oldEBands[(i + c * m.nbEBands) as usize])
                + prev[c as usize]
                + q;
            oldEBands[(i + c * m.nbEBands) as usize] = tmp;
            prev[c as usize] = prev[c as usize] + q - MULT16_32_Q15(beta, q);
            c += 1;
            if c >= C {
                break;
            }
        }
    }
}

pub fn unquant_fine_energy(
    m: &CELTMode,
    start: i32,
    end: i32,
    oldEBands: &mut [celt_glog],
    prev_quant: Option<&[i32]>,
    extra_quant: &[i32],
    dec: &mut ec_ctx,
    C: i32,
) {
    let mut c: i32;
    /* Decode finer resolution */
    for i in start..end {
        let extra: opus_int16;
        let prev: opus_int16;
        extra = extra_quant[i as usize] as opus_int16;
        if extra_quant[i as usize] <= 0 {
            continue;
        }
        if dec.ec_tell() + C * extra_quant[i as usize] > (dec.storage * 8) as opus_int32 {
            continue;
        }
        prev = match prev_quant {
            Some(p) => p[i as usize] as opus_int16,
            None => 0,
        };
        c = 0;
        loop {
            let q2: i32;
            q2 = ec_dec_bits(dec, extra as u32) as i32;
            let mut offset: celt_glog = (q2 as celt_glog + 0.5f32)
                * (1i32 << (14 - extra as i32)) as celt_glog
                * (1.0f32 / 16384 as celt_glog)
                - 0.5f32;
            offset *= (1i32 << (14 - prev as i32)) as celt_glog * (1.0f32 / 16384 as celt_glog);
            oldEBands[(i + c * m.nbEBands) as usize] += offset;
            c += 1;
            if c >= C {
                break;
            }
        }
    }
}

pub fn unquant_energy_finalise(
    m: &CELTMode,
    start: i32,
    end: i32,
    mut oldEBands: Option<&mut [celt_glog]>,
    fine_quant: &[i32],
    fine_priority: &[i32],
    bits_left_: i32,
    dec: &mut ec_ctx,
    C: i32,
) {
    let mut prio: i32;
    let mut c: i32;
    let mut bits_left: i32 = bits_left_;

    /* Use up the remaining bits */
    prio = 0;
    loop {
        let mut i: i32 = start;
        while i < end && bits_left >= C {
            if fine_quant[i as usize] >= MAX_FINE_BITS || fine_priority[i as usize] != prio {
                i += 1;
                continue;
            }
            c = 0;
            loop {
                let q2: i32;
                let offset: celt_glog;
                q2 = ec_dec_bits(dec, 1) as i32;
                offset = (q2 as celt_glog - 0.5f32)
                    * (1i32 << (14 - fine_quant[i as usize] - 1)) as celt_glog
                    * (1.0f32 / 16384 as celt_glog);
                if let Some(ref mut be) = oldEBands {
                    be[(i + c * m.nbEBands) as usize] += offset;
                }
                bits_left -= 1;
                c += 1;
                if c >= C {
                    break;
                }
            }
            i += 1;
        }
        prio += 1;
        if prio >= 2 {
            break;
        }
    }
}

pub fn amp2Log2(
    m: &CELTMode,
    effEnd: i32,
    end: i32,
    bandE: &[celt_ener],
    bandLogE: &mut [celt_glog],
    C: i32,
) {
    let mut c: i32;
    c = 0;
    loop {
        for i in 0..effEnd {
            bandLogE[(i + c * m.nbEBands) as usize] = celt_log2_db(bandE[(i + c * m.nbEBands) as usize])
                - SHL32(eMeans[i as usize], DB_SHIFT - 4);
        }
        for i in effEnd..end {
            bandLogE[(c * m.nbEBands + i) as usize] = -GCONST(14.0f32);
        }
        c += 1;
        if c >= C {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::modes::opus_custom_mode_create;

    /// The float tables must match the C initialisers exactly.
    #[test]
    fn tables_are_exact() {
        assert_eq!(pred_coef[0].to_bits(), 0.8984375f32.to_bits());
        assert_eq!(pred_coef[1].to_bits(), 0.796875f32.to_bits());
        assert_eq!(pred_coef[2].to_bits(), 0.6484375f32.to_bits());
        assert_eq!(pred_coef[3].to_bits(), 0.5f32.to_bits());
        assert_eq!(beta_coef[0].to_bits(), ((30147f64 / 32768.) as f32).to_bits());
        assert_eq!(beta_coef[1].to_bits(), ((22282f64 / 32768.) as f32).to_bits());
        assert_eq!(beta_coef[2].to_bits(), ((12124f64 / 32768.) as f32).to_bits());
        assert_eq!(beta_coef[3].to_bits(), ((6554f64 / 32768.) as f32).to_bits());
        assert_eq!(beta_intra.to_bits(), ((4915f64 / 32768.) as f32).to_bits());
        assert_eq!(eMeans.len(), 25);
        assert_eq!(eMeans[0], 6.4375);
        assert_eq!(eMeans[19], 4.4375);
        assert_eq!(eMeans[24], 3.75);
        for lm in 0..4 {
            for intra in 0..2 {
                assert_eq!(e_prob_model[lm][intra].len(), 42);
            }
        }
    }

    fn mode() -> Box<CELTMode> {
        let mut err = 0;
        opus_custom_mode_create(48000, 960, &mut err).unwrap()
    }

    /// `amp2Log2` on `bandE == 1`: `celt_log2(1) == 0` exactly, so the output
    /// is exactly `-eMeans[i]`.
    #[test]
    fn amp2log2_sanity() {
        let m = mode();
        let nb = m.nbEBands as usize;
        let bandE = vec![1f32; 2 * nb];
        let mut bandLogE = vec![123.0f32; 2 * nb];
        amp2Log2(&m, m.nbEBands, m.nbEBands, &bandE, &mut bandLogE, 2);
        for c in 0..2 {
            for i in 0..nb {
                assert_eq!(bandLogE[c * nb + i], 0.0 - eMeans[i], "c={c} i={i}");
            }
        }
    }

    /// `amp2Log2` writes `-14` in the `[effEnd, end)` tail.
    #[test]
    fn amp2log2_tail_is_minus_14() {
        let m = mode();
        let nb = m.nbEBands as usize;
        let bandE = vec![1f32; 2 * nb];
        let mut bandLogE = vec![123.0f32; 2 * nb];
        amp2Log2(&m, 5, nb as i32, &bandE, &mut bandLogE, 2);
        for c in 0..2 {
            for i in 5..nb {
                assert_eq!(bandLogE[c * nb + i], -14.0);
            }
            for i in 0..5 {
                assert_eq!(bandLogE[c * nb + i], 0.0 - eMeans[i]);
            }
        }
    }

    /// A coarse/fine/finalise round trip must leave the encoder and the decoder
    /// with bit-identical `oldEBands`.
    #[test]
    fn energy_round_trip_is_self_consistent() {
        let m = mode();
        let nb = m.nbEBands as usize;
        let (start, end, C, LM) = (0i32, m.nbEBands, 2i32, 0i32);
        let n = (C * m.nbEBands) as usize;
        let mut state: u32 = 0x1234_5678;
        let mut rnd = move || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            state
        };
        let mut eBands = vec![0f32; n];
        let mut oldEBands = vec![0f32; n];
        for i in 0..n {
            eBands[i] = (rnd() % 20000) as f32 / 1000.0 - 10.0;
            oldEBands[i] = (rnd() % 20000) as f32 / 1000.0 - 10.0;
        }
        let mut dec_old = oldEBands.clone();

        let mut extra_quant = vec![0i32; nb];
        let mut fine_priority = vec![0i32; nb];
        for i in 0..nb {
            extra_quant[i] = (rnd() % 4) as i32;
            fine_priority[i] = (rnd() % 2) as i32;
        }

        let storage: u32 = 400;
        let mut enc = crate::celt::entenc::ec_enc_init(storage);
        let mut error = vec![0f32; n];
        let mut delayed_intra: f32 = 0.0;
        quant_coarse_energy(
            &m, start, end, end, &eBands, &mut oldEBands, storage * 8, &mut error, &mut enc, C, LM,
            400, 0, &mut delayed_intra, 1, 0, 0,
        );
        quant_fine_energy(
            &m,
            start,
            end,
            &mut oldEBands,
            &mut error,
            None,
            &extra_quant,
            &mut enc,
            C,
        );
        let bits_left = (storage * 8) as i32 - enc.ec_tell();
        quant_energy_finalise(
            &m,
            start,
            end,
            Some(&mut oldEBands),
            &mut error,
            &extra_quant,
            &fine_priority,
            bits_left,
            &mut enc,
            C,
        );
        crate::celt::entenc::ec_enc_done(&mut enc);
        assert_eq!(enc.ec_get_error(), 0);

        let mut dec = crate::celt::entdec::ec_dec_init(enc.ec_get_buffer());
        let budget = (dec.storage * 8) as i32;
        let intra = if dec.ec_tell() + 3 <= budget {
            ec_dec_bit_logp(&mut dec, 3)
        } else {
            0
        };
        unquant_coarse_energy(&m, start, end, &mut dec_old, intra, &mut dec, C, LM);
        unquant_fine_energy(&m, start, end, &mut dec_old, None, &extra_quant, &mut dec, C);
        let bits_left = (dec.storage * 8) as i32 - dec.ec_tell();
        unquant_energy_finalise(
            &m,
            start,
            end,
            Some(&mut dec_old),
            &extra_quant,
            &fine_priority,
            bits_left,
            &mut dec,
            C,
        );
        assert_eq!(dec.ec_get_error(), 0);
        assert_eq!(oldEBands, dec_old);
    }
}

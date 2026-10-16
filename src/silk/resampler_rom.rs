//! Mirror of `silk/resampler_rom.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! IIR/FIR polyphase resampler coefficients ('Total size: 179 Words').
//!
//! Original C copyright notice (this file is a derived work):
// /***********************************************************************
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
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

use crate::silk::typedef::*;
use crate::types::*;

// `silk/resampler_private.h` (there is no Rust module for that header):
/// `RESAMPLER_MAX_BATCH_SIZE_MS`
pub const RESAMPLER_MAX_BATCH_SIZE_MS: opus_int = 10;
/// `RESAMPLER_MAX_FS_KHZ`
pub const RESAMPLER_MAX_FS_KHZ: opus_int = 48;
/// `RESAMPLER_MAX_BATCH_SIZE_IN`
pub const RESAMPLER_MAX_BATCH_SIZE_IN: opus_int = RESAMPLER_MAX_BATCH_SIZE_MS * RESAMPLER_MAX_FS_KHZ;

// `RESAMPLER_DOWN_ORDER_FIR0`, `RESAMPLER_DOWN_ORDER_FIR1`,
// `RESAMPLER_DOWN_ORDER_FIR2` and `RESAMPLER_ORDER_FIR_12` come from
// `silk/resampler_rom.h`:
/// `RESAMPLER_DOWN_ORDER_FIR0`
pub const RESAMPLER_DOWN_ORDER_FIR0: opus_int = 18;
/// `RESAMPLER_DOWN_ORDER_FIR1`
pub const RESAMPLER_DOWN_ORDER_FIR1: opus_int = 24;
/// `RESAMPLER_DOWN_ORDER_FIR2`
pub const RESAMPLER_DOWN_ORDER_FIR2: opus_int = 36;
/// `RESAMPLER_ORDER_FIR_12`
pub const RESAMPLER_ORDER_FIR_12: opus_int = 8;

/* Tables for 2x downsampler (`silk/resampler_rom.h`, static in the C) */
/// `static const opus_int16 silk_resampler_down2_0 = 9872;`
pub const silk_resampler_down2_0: opus_int16 = 9872;
/// `static const opus_int16 silk_resampler_down2_1 = 39809 - 65536;` (= -25727)
pub const silk_resampler_down2_1: opus_int16 = -25727;

/* Tables for 2x upsampler, high quality (`silk/resampler_rom.h`, static in the C) */
/// `static const opus_int16 silk_resampler_up2_hq_0[ 3 ]`
/// (`39083 - 65536` = -26453)
pub static silk_resampler_up2_hq_0: [opus_int16; 3] = [1746, 14986, -26453];
/// `static const opus_int16 silk_resampler_up2_hq_1[ 3 ]`
/// (`55542 - 65536` = -9994)
pub static silk_resampler_up2_hq_1: [opus_int16; 3] = [6854, 25769, -9994];


/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_3_4_COEFS[2 + 3 * RESAMPLER_DOWN_ORDER_FIR0 / 2] (= 29)`.
pub static silk_Resampler_3_4_COEFS: [i16; 29] = [
    -20694, -13867, -49, 64, 17, -157, 353, -496, 163, 11047, 22205, -39, 6, 91, -170, 186, 23, -896, 6336, 19928, -19, -36, 102, -89, -24, 328, -951, 2568, 15909,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_2_3_COEFS[2 + 2 * RESAMPLER_DOWN_ORDER_FIR0 / 2] (= 20)`.
pub static silk_Resampler_2_3_COEFS: [i16; 20] = [
    -14457, -14019, 64, 128, -122, 36, 310, -768, 584, 9267, 17733, 12, 128, 18, -142, 288, -117, -865, 4123, 14459,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_1_2_COEFS[2 + RESAMPLER_DOWN_ORDER_FIR1 / 2] (= 14)`.
pub static silk_Resampler_1_2_COEFS: [i16; 14] = [
    616, -14323, -10, 39, 58, -46, -84, 120, 184, -315, -541, 1284, 5380, 9024,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_1_3_COEFS[2 + RESAMPLER_DOWN_ORDER_FIR2 / 2] (= 20)`.
pub static silk_Resampler_1_3_COEFS: [i16; 20] = [
    16102, -15162, -13, 0, 20, 26, 5, -31, -43, -4, 65, 90, 7, -157, -248, -44, 593, 1583, 2612, 3271,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_1_4_COEFS[2 + RESAMPLER_DOWN_ORDER_FIR2 / 2] (= 20)`.
pub static silk_Resampler_1_4_COEFS: [i16; 20] = [
    22500, -15099, 3, -14, -20, -15, 2, 25, 37, 25, -16, -71, -107, -79, 50, 292, 623, 982, 1288, 1464,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_1_6_COEFS[2 + RESAMPLER_DOWN_ORDER_FIR2 / 2] (= 20)`.
pub static silk_Resampler_1_6_COEFS: [i16; 20] = [
    27540, -15257, 17, 12, 8, 1, -10, -22, -30, -32, -22, 3, 44, 100, 168, 243, 317, 381, 429, 455,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_Resampler_2_3_COEFS_LQ[2 + 2 * 2] (= 6)`.
pub static silk_Resampler_2_3_COEFS_LQ: [i16; 6] = [
    -2797, -6507, 4697, 10739, 1567, 8276,
];

/// C: `silk_DWORD_ALIGN const opus_int16 silk_resampler_frac_FIR_12[12][RESAMPLER_ORDER_FIR_12 / 2] (= [12][4])`.
pub static silk_resampler_frac_FIR_12: [[i16; 4]; 12] = [
    [189, -600, 617, 30567],
    [117, -159, -1070, 29704],
    [52, 221, -2392, 28276],
    [-4, 529, -3350, 26341],
    [-48, 758, -3956, 23973],
    [-80, 905, -4235, 21254],
    [-99, 972, -4222, 18278],
    [-107, 967, -3957, 15143],
    [-103, 896, -3487, 11950],
    [-91, 773, -2865, 8798],
    [-71, 611, -2143, 5784],
    [-46, 425, -1375, 2996],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_constants_match_the_c_header() {
        assert_eq!(silk_resampler_down2_0, 9872);
        assert_eq!(silk_resampler_down2_1, -25727);
        assert_eq!(silk_resampler_up2_hq_0, [1746, 14986, -26453]);
        assert_eq!(silk_resampler_up2_hq_1, [6854, 25769, -9994]);
    }

    #[test]
    fn table_geometry() {
        assert_eq!(
            silk_Resampler_3_4_COEFS.len(),
            2 + 3 * RESAMPLER_DOWN_ORDER_FIR0 as usize / 2
        );
        assert_eq!(silk_resampler_frac_FIR_12.len(), 12);
        assert_eq!(RESAMPLER_MAX_BATCH_SIZE_IN, 480);
    }
}

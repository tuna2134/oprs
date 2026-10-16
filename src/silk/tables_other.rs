//! Mirror of `silk/tables_other.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The remaining SILK entropy/quantization tables.
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

use crate::silk::define::*;

/// C: `const i16 silk_stereo_pred_quant_Q13[STEREO_QUANT_TAB_SIZE]`.
pub static silk_stereo_pred_quant_Q13: [i16; 16] = [
    -13732, -10050, -8266, -7526, -6500, -5000, -2950, -820, 820, 2950, 5000, 6500, 7526, 8266, 10050, 13732,
];

/// C: `const u8 silk_stereo_pred_joint_iCDF[25]`.
pub static silk_stereo_pred_joint_iCDF: [u8; 25] = [
    249, 247, 246, 245, 244, 234, 210, 202, 201, 200, 197, 174, 82, 59, 56, 55, 54, 46, 22, 12, 11, 10, 9, 7, 0,
];

/// C: `const u8 silk_stereo_only_code_mid_iCDF[2]`.
pub static silk_stereo_only_code_mid_iCDF: [u8; 2] = [
    64, 0,
];

/// C: `const u8 silk_LBRR_flags_2_iCDF[3]`.
pub static silk_LBRR_flags_2_iCDF: [u8; 3] = [
    203, 150, 0,
];

/// C: `const u8 silk_LBRR_flags_3_iCDF[7]`.
pub static silk_LBRR_flags_3_iCDF: [u8; 7] = [
    215, 195, 166, 125, 110, 82, 0,
];

/// C: `const u8 silk_lsb_iCDF[2]`.
pub static silk_lsb_iCDF: [u8; 2] = [
    120, 0,
];

/// C: `const u8 silk_LTPscale_iCDF[3]`.
pub static silk_LTPscale_iCDF: [u8; 3] = [
    128, 64, 0,
];

/// C: `const u8 silk_type_offset_VAD_iCDF[4]`.
pub static silk_type_offset_VAD_iCDF: [u8; 4] = [
    232, 158, 10, 0,
];

/// C: `const u8 silk_type_offset_no_VAD_iCDF[2]`.
pub static silk_type_offset_no_VAD_iCDF: [u8; 2] = [
    230, 0,
];

/// C: `const u8 silk_NLSF_interpolation_factor_iCDF[5]`.
pub static silk_NLSF_interpolation_factor_iCDF: [u8; 5] = [
    243, 221, 192, 181, 0,
];

/// C: `const i16 silk_LTPScales_table_Q14[3]`.
pub static silk_LTPScales_table_Q14: [i16; 3] = [
    15565, 12288, 8192,
];

/// C: `const u8 silk_uniform3_iCDF[3]`.
pub static silk_uniform3_iCDF: [u8; 3] = [
    171, 85, 0,
];

/// C: `const u8 silk_uniform4_iCDF[4]`.
pub static silk_uniform4_iCDF: [u8; 4] = [
    192, 128, 64, 0,
];

/// C: `const u8 silk_uniform5_iCDF[5]`.
pub static silk_uniform5_iCDF: [u8; 5] = [
    205, 154, 102, 51, 0,
];

/// C: `const u8 silk_uniform6_iCDF[6]`.
pub static silk_uniform6_iCDF: [u8; 6] = [
    213, 171, 128, 85, 43, 0,
];

/// C: `const u8 silk_uniform8_iCDF[8]`.
pub static silk_uniform8_iCDF: [u8; 8] = [
    224, 192, 160, 128, 96, 64, 32, 0,
];

/// C: `const u8 silk_NLSF_EXT_iCDF[7]`.
pub static silk_NLSF_EXT_iCDF: [u8; 7] = [
    100, 40, 16, 7, 3, 1, 0,
];

/// C: `const i32 silk_Transition_LP_B_Q28[TRANSITION_INT_NUM][TRANSITION_NB] (= [5][3])`.
pub static silk_Transition_LP_B_Q28: [[i32; 3]; 5] = [
    [250767114, 501534038, 250767114],
    [209867381, 419732057, 209867381],
    [170987846, 341967853, 170987846],
    [131531482, 263046905, 131531482],
    [89306658, 178584282, 89306658],
];

/// C: `const i32 silk_Transition_LP_A_Q28[TRANSITION_INT_NUM][TRANSITION_NA] (= [5][2])`.
pub static silk_Transition_LP_A_Q28: [[i32; 2]; 5] = [
    [506393414, 239854379],
    [411067935, 169683996],
    [306733530, 116694253],
    [185807084, 77959395],
    [35497197, 57401098],
];

/// C: `const u8 * const silk_LBRR_flags_iCDF_ptr[2]`
pub static silk_LBRR_flags_iCDF_ptr: [&[u8]; 2] =
    [&silk_LBRR_flags_2_iCDF, &silk_LBRR_flags_3_iCDF];

/// C: `const i16 silk_Quantization_Offsets_Q10[2][2] = { { OFFSET_UVL_Q10, OFFSET_UVH_Q10 }, { OFFSET_VL_Q10, OFFSET_VH_Q10 } };`
pub static silk_Quantization_Offsets_Q10: [[i16; 2]; 2] = [
    [OFFSET_UVL_Q10 as i16, OFFSET_UVH_Q10 as i16],
    [OFFSET_VL_Q10 as i16, OFFSET_VH_Q10 as i16],
];

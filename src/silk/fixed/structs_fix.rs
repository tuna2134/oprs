//! Mirror of `silk/fixed/structs_FIX.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The fixed-point encoder structures, plus `silk_decoder` (which the C declares in `dec_API.c`).  The pinned build is floating point, so the encoder itself uses `silk_encoder_state_FLP`; these definitions are kept because they are part of the SILK foundation and the decoder-side super-structure is build-independent.
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
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

use crate::silk::structs::silk_default;

// ---------------------------------------------------------------------------
// Noise shaping analysis state
// ---------------------------------------------------------------------------

/// `silk_shape_state_FIX`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_shape_state_FIX {
    pub LastGainIndex: opus_int8,
    pub HarmBoost_smth_Q16: opus_int32,
    pub HarmShapeGain_smth_Q16: opus_int32,
    pub Tilt_smth_Q16: opus_int32,
}

silk_default!(silk_shape_state_FIX {
    LastGainIndex: 0,
    HarmBoost_smth_Q16: 0,
    HarmShapeGain_smth_Q16: 0,
    Tilt_smth_Q16: 0,
});

// ---------------------------------------------------------------------------
// Encoder state FIX
// ---------------------------------------------------------------------------

/// `silk_encoder_state_FIX`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_encoder_state_FIX {
    /// Common struct, shared with floating-point code
    pub sCmn: silk_encoder_state,
    /// Shape state
    pub sShape: silk_shape_state_FIX,

    /// Buffer for find pitch and noise shape analysis
    pub x_buf: [opus_int16; (2 * MAX_FRAME_LENGTH + LA_SHAPE_MAX) as usize],
    /// Normalized correlation from pitch lag estimator
    pub LTPCorr_Q15: opus_int,
    pub resNrgSmth: opus_int32,
}

silk_default!(silk_encoder_state_FIX {
    sCmn: silk_encoder_state::default(),
    sShape: silk_shape_state_FIX::default(),
    x_buf: [0; (2 * MAX_FRAME_LENGTH + LA_SHAPE_MAX) as usize],
    LTPCorr_Q15: 0,
    resNrgSmth: 0,
});

// ---------------------------------------------------------------------------
// Encoder control FIX
// ---------------------------------------------------------------------------

/// `silk_encoder_control_FIX`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_encoder_control_FIX {
    /// Prediction and coding parameters
    pub Gains_Q16: [opus_int32; MAX_NB_SUBFR as usize],
    pub PredCoef_Q12: [[opus_int16; MAX_LPC_ORDER as usize]; 2],
    pub LTPCoef_Q14: [opus_int16; (LTP_ORDER * MAX_NB_SUBFR) as usize],
    pub LTP_scale_Q14: opus_int,
    pub pitchL: [opus_int; MAX_NB_SUBFR as usize],

    /// Noise shaping parameters
    pub AR_Q13: [opus_int16; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize],
    /// Packs two int16 coefficients per int32 value
    pub LF_shp_Q14: [opus_int32; MAX_NB_SUBFR as usize],
    pub Tilt_Q14: [opus_int; MAX_NB_SUBFR as usize],
    pub HarmShapeGain_Q14: [opus_int; MAX_NB_SUBFR as usize],
    pub Lambda_Q10: opus_int,
    pub input_quality_Q14: opus_int,
    pub coding_quality_Q14: opus_int,

    /// measures
    pub predGain_Q16: opus_int32,
    pub LTPredCodGain_Q7: opus_int,
    /// Residual energy per subframe
    pub ResNrg: [opus_int32; MAX_NB_SUBFR as usize],
    /// Q domain for the residual energy > 0
    pub ResNrgQ: [opus_int; MAX_NB_SUBFR as usize],

    /// Parameters for CBR mode
    pub GainsUnq_Q16: [opus_int32; MAX_NB_SUBFR as usize],
    pub lastGainIndexPrev: opus_int8,
}

silk_default!(silk_encoder_control_FIX {
    Gains_Q16: [0; MAX_NB_SUBFR as usize],
    PredCoef_Q12: [[0; MAX_LPC_ORDER as usize]; 2],
    LTPCoef_Q14: [0; (LTP_ORDER * MAX_NB_SUBFR) as usize],
    LTP_scale_Q14: 0,
    pitchL: [0; MAX_NB_SUBFR as usize],
    AR_Q13: [0; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize],
    LF_shp_Q14: [0; MAX_NB_SUBFR as usize],
    Tilt_Q14: [0; MAX_NB_SUBFR as usize],
    HarmShapeGain_Q14: [0; MAX_NB_SUBFR as usize],
    Lambda_Q10: 0,
    input_quality_Q14: 0,
    coding_quality_Q14: 0,
    predGain_Q16: 0,
    LTPredCodGain_Q7: 0,
    ResNrg: [0; MAX_NB_SUBFR as usize],
    ResNrgQ: [0; MAX_NB_SUBFR as usize],
    GainsUnq_Q16: [0; MAX_NB_SUBFR as usize],
    lastGainIndexPrev: 0,
});

// ---------------------------------------------------------------------------
// Encoder super structure
// ---------------------------------------------------------------------------

/// `silk_encoder` (the `structs_FIX.h` variant).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_encoder {
    pub sStereo: stereo_enc_state,
    pub nBitsUsedLBRR: opus_int32,
    pub nBitsExceeded: opus_int32,
    pub nChannelsAPI: opus_int,
    pub nChannelsInternal: opus_int,
    pub nPrevChannelsInternal: opus_int,
    pub timeSinceSwitchAllowed_ms: opus_int,
    pub allowBandwidthSwitch: opus_int,
    pub prev_decode_only_middle: opus_int,
    /// This needs to be last so we can skip the second state for mono.
    pub state_Fxx: [silk_encoder_state_FIX; ENCODER_NUM_CHANNELS as usize],
}

silk_default!(silk_encoder {
    sStereo: stereo_enc_state::default(),
    nBitsUsedLBRR: 0,
    nBitsExceeded: 0,
    nChannelsAPI: 0,
    nChannelsInternal: 0,
    nPrevChannelsInternal: 0,
    timeSinceSwitchAllowed_ms: 0,
    allowBandwidthSwitch: 0,
    prev_decode_only_middle: 0,
    state_Fxx: [silk_encoder_state_FIX::default(); ENCODER_NUM_CHANNELS as usize],
});

// ---------------------------------------------------------------------------
// `OSCEModel` and the decoder super structure (`silk/dec_API.c`)
// ---------------------------------------------------------------------------

/// `sizeof(OSCEModel)` in the reference configuration (`ENABLE_OSCE` defined,
/// `ENABLE_OSCE_BWE` undefined).
pub const OSCE_MODEL_SIZE: usize = 5448;

/// `OSCEModel` — `dnn/osce_structs.h`.
///
/// Like [`crate::silk::structs::OSCEState`] this is built from the `dnn/`
/// network layer types, so it is kept as an opaque block of exactly the C size
/// and alignment.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct OSCEModel {
    pub _opaque: [u8; OSCE_MODEL_SIZE],
}

silk_default!(OSCEModel {
    _opaque: [0; OSCE_MODEL_SIZE],
});

/// `silk_decoder` — declared in `silk/dec_API.c` (not in a header).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_decoder {
    pub channel_state: [silk_decoder_state; DECODER_NUM_CHANNELS as usize],
    pub sStereo: stereo_dec_state,
    pub nChannelsAPI: opus_int,
    pub nChannelsInternal: opus_int,
    pub prev_decode_only_middle: opus_int,
    /// `#ifdef ENABLE_OSCE`
    pub osce_model: OSCEModel,
}

silk_default!(silk_decoder {
    channel_state: [silk_decoder_state::default(); DECODER_NUM_CHANNELS as usize],
    sStereo: stereo_dec_state::default(),
    nChannelsAPI: 0,
    nChannelsInternal: 0,
    prev_decode_only_middle: 0,
    osce_model: OSCEModel::default(),
});

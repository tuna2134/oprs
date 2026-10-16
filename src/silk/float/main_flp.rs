//! Mirror of `silk/float/main_FLP.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The floating-point encoder structures, which the C declares in `float/structs_FLP.h` (included by this header).  This is the variant the pinned floating-point build uses.
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
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::structs::*;
use crate::silk::structs::silk_default;
use crate::silk::typedef::*;
use crate::types::*;

// `main_FLP.h` renames the build-variant entry points; the type alias is kept
// here, the function names are not needed in Rust.
/// `#define silk_encoder_state_Fxx silk_encoder_state_FLP`
pub use silk_encoder_state_FLP as silk_encoder_state_Fxx;

// ---------------------------------------------------------------------------
// Noise shaping analysis state
// ---------------------------------------------------------------------------

/// `silk_shape_state_FLP`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_shape_state_FLP {
    pub LastGainIndex: opus_int8,
    pub HarmShapeGain_smth: silk_float,
    pub Tilt_smth: silk_float,
}

silk_default!(silk_shape_state_FLP {
    LastGainIndex: 0,
    HarmShapeGain_smth: 0.0,
    Tilt_smth: 0.0,
});

// ---------------------------------------------------------------------------
// Encoder state FLP
// ---------------------------------------------------------------------------

/// `silk_encoder_state_FLP`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_encoder_state_FLP {
    /// Common struct, shared with fixed-point code
    pub sCmn: silk_encoder_state,
    /// Noise shaping state
    pub sShape: silk_shape_state_FLP,

    /// Buffer for find pitch and noise shape analysis
    pub x_buf: [silk_float; (2 * MAX_FRAME_LENGTH + LA_SHAPE_MAX) as usize],
    /// Normalized correlation from pitch lag estimator
    pub LTPCorr: silk_float,
}

silk_default!(silk_encoder_state_FLP {
    sCmn: silk_encoder_state::default(),
    sShape: silk_shape_state_FLP::default(),
    x_buf: [0.0; (2 * MAX_FRAME_LENGTH + LA_SHAPE_MAX) as usize],
    LTPCorr: 0.0,
});

// ---------------------------------------------------------------------------
// Encoder control FLP
// ---------------------------------------------------------------------------

/// `silk_encoder_control_FLP`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_encoder_control_FLP {
    /// Prediction and coding parameters
    pub Gains: [silk_float; MAX_NB_SUBFR as usize],
    /// holds interpolated and final coefficients
    pub PredCoef: [[silk_float; MAX_LPC_ORDER as usize]; 2],
    pub LTPCoef: [silk_float; (LTP_ORDER * MAX_NB_SUBFR) as usize],
    pub LTP_scale: silk_float,
    pub pitchL: [opus_int; MAX_NB_SUBFR as usize],

    /// Noise shaping parameters
    pub AR: [silk_float; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize],
    pub LF_MA_shp: [silk_float; MAX_NB_SUBFR as usize],
    pub LF_AR_shp: [silk_float; MAX_NB_SUBFR as usize],
    pub Tilt: [silk_float; MAX_NB_SUBFR as usize],
    pub HarmShapeGain: [silk_float; MAX_NB_SUBFR as usize],
    pub Lambda: silk_float,
    pub input_quality: silk_float,
    pub coding_quality: silk_float,

    /// Measures
    pub predGain: silk_float,
    pub LTPredCodGain: silk_float,
    /// Residual energy per subframe
    pub ResNrg: [silk_float; MAX_NB_SUBFR as usize],

    /// Parameters for CBR mode
    pub GainsUnq_Q16: [opus_int32; MAX_NB_SUBFR as usize],
    pub lastGainIndexPrev: opus_int8,
}

silk_default!(silk_encoder_control_FLP {
    Gains: [0.0; MAX_NB_SUBFR as usize],
    PredCoef: [[0.0; MAX_LPC_ORDER as usize]; 2],
    LTPCoef: [0.0; (LTP_ORDER * MAX_NB_SUBFR) as usize],
    LTP_scale: 0.0,
    pitchL: [0; MAX_NB_SUBFR as usize],
    AR: [0.0; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize],
    LF_MA_shp: [0.0; MAX_NB_SUBFR as usize],
    LF_AR_shp: [0.0; MAX_NB_SUBFR as usize],
    Tilt: [0.0; MAX_NB_SUBFR as usize],
    HarmShapeGain: [0.0; MAX_NB_SUBFR as usize],
    Lambda: 0.0,
    input_quality: 0.0,
    coding_quality: 0.0,
    predGain: 0.0,
    LTPredCodGain: 0.0,
    ResNrg: [0.0; MAX_NB_SUBFR as usize],
    GainsUnq_Q16: [0; MAX_NB_SUBFR as usize],
    lastGainIndexPrev: 0,
});

// ---------------------------------------------------------------------------
// Encoder super structure
// ---------------------------------------------------------------------------

/// `silk_encoder` (the `structs_FLP.h` variant, used by the pinned build).
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
    pub state_Fxx: [silk_encoder_state_FLP; ENCODER_NUM_CHANNELS as usize],
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
    state_Fxx: [silk_encoder_state_FLP::default(); ENCODER_NUM_CHANNELS as usize],
});

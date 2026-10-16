//! Mirror of `silk/structs.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Every SILK state structure.  Pointer members become slices / `&'static` references (convention 6); the OSCE sub-state is opaque because `dnn/` is out of scope.  `silk_resampler_state_struct` lives in `silk/resampler_structs.h` and is included here because both codec states embed it.
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
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_memset(x, 0, sizeof(*x))` — the C initialises every state structure
/// with zeros, so every ported structure gets a `Default` implementation that
/// reproduces that.  Arrays longer than 32 elements cannot use the derived
/// `Default`, hence this helper.
macro_rules! silk_default {
    ($name:ident { $($field:ident : $value:expr),* $(,)? }) => {
        impl Default for $name {
            fn default() -> Self {
                $name { $($field: $value),* }
            }
        }
    };
}
pub(crate) use silk_default;

// ---------------------------------------------------------------------------
// `silk/resampler_structs.h`
// ---------------------------------------------------------------------------

/// `SILK_RESAMPLER_MAX_FIR_ORDER`
pub const SILK_RESAMPLER_MAX_FIR_ORDER: opus_int = 36;
/// `SILK_RESAMPLER_MAX_IIR_ORDER`
pub const SILK_RESAMPLER_MAX_IIR_ORDER: opus_int = 6;

/// `union { opus_int32 i32[36]; opus_int16 i16[36]; } sFIR;`
///
/// The C reads and writes the same storage through both views, so a Rust union
/// is the exact counterpart (accessing the non-active variant is `unsafe`,
/// exactly as it is undefined-ish in C).
#[repr(C)]
#[derive(Clone, Copy)]
pub union silk_resampler_sFIR {
    pub i32: [opus_int32; SILK_RESAMPLER_MAX_FIR_ORDER as usize],
    pub i16: [opus_int16; SILK_RESAMPLER_MAX_FIR_ORDER as usize],
}

/// `typedef struct _silk_resampler_state_struct`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_resampler_state_struct {
    /// this must be the first element of this struct
    pub sIIR: [opus_int32; SILK_RESAMPLER_MAX_IIR_ORDER as usize],
    pub sFIR: silk_resampler_sFIR,
    pub delayBuf: [opus_int16; 96],
    pub resampler_function: opus_int,
    pub batchSize: opus_int,
    pub invRatio_Q16: opus_int32,
    pub FIR_Order: opus_int,
    pub FIR_Fracs: opus_int,
    pub Fs_in_kHz: opus_int,
    pub Fs_out_kHz: opus_int,
    pub inputDelay: opus_int,
    /// `const opus_int16 *Coefs` — points into `resampler_rom.rs`.
    pub Coefs: &'static [opus_int16],
}

silk_default!(silk_resampler_state_struct {
    sIIR: [0; SILK_RESAMPLER_MAX_IIR_ORDER as usize],
    sFIR: silk_resampler_sFIR { i32: [0; SILK_RESAMPLER_MAX_FIR_ORDER as usize] },
    delayBuf: [0; 96],
    resampler_function: 0,
    batchSize: 0,
    invRatio_Q16: 0,
    FIR_Order: 0,
    FIR_Fracs: 0,
    Fs_in_kHz: 0,
    Fs_out_kHz: 0,
    inputDelay: 0,
    Coefs: &[],
});

// ---------------------------------------------------------------------------
// Noise shaping quantization state
// ---------------------------------------------------------------------------

/// `silk_nsq_state`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_nsq_state {
    /// Buffer for quantized output signal
    pub xq: [opus_int16; (2 * MAX_FRAME_LENGTH) as usize],
    pub sLTP_shp_Q14: [opus_int32; (2 * MAX_FRAME_LENGTH) as usize],
    pub sLPC_Q14: [opus_int32; (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize],
    pub sAR2_Q14: [opus_int32; MAX_SHAPE_LPC_ORDER as usize],
    pub sLF_AR_shp_Q14: opus_int32,
    pub sDiff_shp_Q14: opus_int32,
    pub lagPrev: opus_int,
    pub sLTP_buf_idx: opus_int,
    pub sLTP_shp_buf_idx: opus_int,
    pub rand_seed: opus_int32,
    pub prev_gain_Q16: opus_int32,
    pub rewhite_flag: opus_int,
}

silk_default!(silk_nsq_state {
    xq: [0; (2 * MAX_FRAME_LENGTH) as usize],
    sLTP_shp_Q14: [0; (2 * MAX_FRAME_LENGTH) as usize],
    sLPC_Q14: [0; (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize],
    sAR2_Q14: [0; MAX_SHAPE_LPC_ORDER as usize],
    sLF_AR_shp_Q14: 0,
    sDiff_shp_Q14: 0,
    lagPrev: 0,
    sLTP_buf_idx: 0,
    sLTP_shp_buf_idx: 0,
    rand_seed: 0,
    prev_gain_Q16: 0,
    rewhite_flag: 0,
});

// ---------------------------------------------------------------------------
// VAD state
// ---------------------------------------------------------------------------

/// `silk_VAD_state`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_VAD_state {
    /// Analysis filterbank state: 0-8 kHz
    pub AnaState: [opus_int32; 2],
    /// Analysis filterbank state: 0-4 kHz
    pub AnaState1: [opus_int32; 2],
    /// Analysis filterbank state: 0-2 kHz
    pub AnaState2: [opus_int32; 2],
    /// Subframe energies
    pub XnrgSubfr: [opus_int32; VAD_N_BANDS as usize],
    /// Smoothed energy level in each band
    pub NrgRatioSmth_Q8: [opus_int32; VAD_N_BANDS as usize],
    /// State of differentiator in the lowest band
    pub HPstate: opus_int16,
    /// Noise energy level in each band
    pub NL: [opus_int32; VAD_N_BANDS as usize],
    /// Inverse noise energy level in each band
    pub inv_NL: [opus_int32; VAD_N_BANDS as usize],
    /// Noise level estimator bias/offset
    pub NoiseLevelBias: [opus_int32; VAD_N_BANDS as usize],
    /// Frame counter used in the initial phase
    pub counter: opus_int32,
}

silk_default!(silk_VAD_state {
    AnaState: [0; 2],
    AnaState1: [0; 2],
    AnaState2: [0; 2],
    XnrgSubfr: [0; VAD_N_BANDS as usize],
    NrgRatioSmth_Q8: [0; VAD_N_BANDS as usize],
    HPstate: 0,
    NL: [0; VAD_N_BANDS as usize],
    inv_NL: [0; VAD_N_BANDS as usize],
    NoiseLevelBias: [0; VAD_N_BANDS as usize],
    counter: 0,
});

// ---------------------------------------------------------------------------
// Variable cut-off low-pass filter state
// ---------------------------------------------------------------------------

/// `silk_LP_state`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_LP_state {
    /// Low pass filter state
    pub In_LP_State: [opus_int32; 2],
    /// Counter which is mapped to a cut-off frequency
    pub transition_frame_no: opus_int32,
    /// Operating mode, <0: switch down, >0: switch up; 0: do nothing
    pub mode: opus_int,
    /// If non-zero, holds the last sampling rate before a bandwidth switching reset.
    pub saved_fs_kHz: opus_int32,
}

silk_default!(silk_LP_state {
    In_LP_State: [0; 2],
    transition_frame_no: 0,
    mode: 0,
    saved_fs_kHz: 0,
});

// ---------------------------------------------------------------------------
// NLSF codebook
// ---------------------------------------------------------------------------

/// Structure containing NLSF codebook (`silk_NLSF_CB_struct`).
///
/// The C members are `const T *` pointers into the codebook tables; each one
/// carries its own length in the Rust version.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_NLSF_CB_struct {
    pub nVectors: opus_int16,
    pub order: opus_int16,
    pub quantStepSize_Q16: opus_int16,
    pub invQuantStepSize_Q6: opus_int16,
    /// `const opus_uint8 *CB1_NLSF_Q8`
    pub CB1_NLSF_Q8: &'static [opus_uint8],
    /// `const opus_int16 *CB1_Wght_Q9`
    pub CB1_Wght_Q9: &'static [opus_int16],
    /// `const opus_uint8 *CB1_iCDF`
    pub CB1_iCDF: &'static [opus_uint8],
    /// `const opus_uint8 *pred_Q8`
    pub pred_Q8: &'static [opus_uint8],
    /// `const opus_uint8 *ec_sel`
    pub ec_sel: &'static [opus_uint8],
    /// `const opus_uint8 *ec_iCDF`
    pub ec_iCDF: &'static [opus_uint8],
    /// `const opus_uint8 *ec_Rates_Q5`
    pub ec_Rates_Q5: &'static [opus_uint8],
    /// `const opus_int16 *deltaMin_Q15`
    pub deltaMin_Q15: &'static [opus_int16],
}

silk_default!(silk_NLSF_CB_struct {
    nVectors: 0,
    order: 0,
    quantStepSize_Q16: 0,
    invQuantStepSize_Q6: 0,
    CB1_NLSF_Q8: &[],
    CB1_Wght_Q9: &[],
    CB1_iCDF: &[],
    pred_Q8: &[],
    ec_sel: &[],
    ec_iCDF: &[],
    ec_Rates_Q5: &[],
    deltaMin_Q15: &[],
});

// ---------------------------------------------------------------------------
// Stereo states
// ---------------------------------------------------------------------------

/// `stereo_enc_state`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct stereo_enc_state {
    pub pred_prev_Q13: [opus_int16; 2],
    pub sMid: [opus_int16; 2],
    pub sSide: [opus_int16; 2],
    pub mid_side_amp_Q0: [opus_int32; 4],
    pub smth_width_Q14: opus_int16,
    pub width_prev_Q14: opus_int16,
    pub silent_side_len: opus_int16,
    pub predIx: [[[opus_int8; 3]; 2]; MAX_FRAMES_PER_PACKET as usize],
    pub mid_only_flags: [opus_int8; MAX_FRAMES_PER_PACKET as usize],
}

silk_default!(stereo_enc_state {
    pred_prev_Q13: [0; 2],
    sMid: [0; 2],
    sSide: [0; 2],
    mid_side_amp_Q0: [0; 4],
    smth_width_Q14: 0,
    width_prev_Q14: 0,
    silent_side_len: 0,
    predIx: [[[0; 3]; 2]; MAX_FRAMES_PER_PACKET as usize],
    mid_only_flags: [0; MAX_FRAMES_PER_PACKET as usize],
});

/// `stereo_dec_state`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct stereo_dec_state {
    pub pred_prev_Q13: [opus_int16; 2],
    pub sMid: [opus_int16; 2],
    pub sSide: [opus_int16; 2],
}

silk_default!(stereo_dec_state {
    pred_prev_Q13: [0; 2],
    sMid: [0; 2],
    sSide: [0; 2],
});

/// `SideInfoIndices`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SideInfoIndices {
    pub GainsIndices: [opus_int8; MAX_NB_SUBFR as usize],
    pub LTPIndex: [opus_int8; MAX_NB_SUBFR as usize],
    pub NLSFIndices: [opus_int8; (MAX_LPC_ORDER + 1) as usize],
    pub lagIndex: opus_int16,
    pub contourIndex: opus_int8,
    pub signalType: opus_int8,
    pub quantOffsetType: opus_int8,
    pub NLSFInterpCoef_Q2: opus_int8,
    pub PERIndex: opus_int8,
    pub LTP_scaleIndex: opus_int8,
    pub Seed: opus_int8,
}

silk_default!(SideInfoIndices {
    GainsIndices: [0; MAX_NB_SUBFR as usize],
    LTPIndex: [0; MAX_NB_SUBFR as usize],
    NLSFIndices: [0; (MAX_LPC_ORDER + 1) as usize],
    lagIndex: 0,
    contourIndex: 0,
    signalType: 0,
    quantOffsetType: 0,
    NLSFInterpCoef_Q2: 0,
    PERIndex: 0,
    LTP_scaleIndex: 0,
    Seed: 0,
});

// ---------------------------------------------------------------------------
// Encoder state
// ---------------------------------------------------------------------------

/// `silk_encoder_state`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_encoder_state {
    /// High pass filter state
    pub In_HP_State: [opus_int32; 2],
    /// State of first smoother
    pub variable_HP_smth1_Q15: opus_int32,
    /// State of second smoother
    pub variable_HP_smth2_Q15: opus_int32,
    /// Low pass filter state
    pub sLP: silk_LP_state,
    /// Voice activity detector state
    pub sVAD: silk_VAD_state,
    /// Noise Shape Quantizer State
    pub sNSQ: silk_nsq_state,
    /// Previously quantized NLSF vector
    pub prev_NLSFq_Q15: [opus_int16; MAX_LPC_ORDER as usize],
    /// Speech activity
    pub speech_activity_Q8: opus_int,
    /// Flag indicating that switching of internal bandwidth is allowed
    pub allow_bandwidth_switch: opus_int,
    pub LBRRprevLastGainIndex: opus_int8,
    pub prevSignalType: opus_int8,
    pub prevLag: opus_int,
    pub pitch_LPC_win_length: opus_int,
    /// Highest possible pitch lag (samples)
    pub max_pitch_lag: opus_int,
    /// API sampling frequency (Hz)
    pub API_fs_Hz: opus_int32,
    /// Previous API sampling frequency (Hz)
    pub prev_API_fs_Hz: opus_int32,
    /// Maximum internal sampling frequency (Hz)
    pub maxInternal_fs_Hz: opus_int,
    /// Minimum internal sampling frequency (Hz)
    pub minInternal_fs_Hz: opus_int,
    /// Soft request for internal sampling frequency (Hz)
    pub desiredInternal_fs_Hz: opus_int,
    /// Internal sampling frequency (kHz)
    pub fs_kHz: opus_int,
    /// Number of 5 ms subframes in a frame
    pub nb_subfr: opus_int,
    /// Frame length (samples)
    pub frame_length: opus_int,
    /// Subframe length (samples)
    pub subfr_length: opus_int,
    /// Length of LTP memory
    pub ltp_mem_length: opus_int,
    /// Look-ahead for pitch analysis (samples)
    pub la_pitch: opus_int,
    /// Look-ahead for noise shape analysis (samples)
    pub la_shape: opus_int,
    /// Window length for noise shape analysis (samples)
    pub shapeWinLength: opus_int,
    /// Target bitrate (bps)
    pub TargetRate_bps: opus_int32,
    /// Number of milliseconds to put in each packet
    pub PacketSize_ms: opus_int,
    /// Packet loss rate measured by farend
    pub PacketLoss_perc: opus_int,
    pub frameCounter: opus_int32,
    /// Complexity setting
    pub Complexity: opus_int,
    /// Number of states in delayed decision quantization
    pub nStatesDelayedDecision: opus_int,
    /// Flag for using NLSF interpolation
    pub useInterpolatedNLSFs: opus_int,
    /// Filter order for noise shaping filters
    pub shapingLPCOrder: opus_int,
    /// Filter order for prediction filters
    pub predictLPCOrder: opus_int,
    /// Complexity level for pitch estimator
    pub pitchEstimationComplexity: opus_int,
    /// Whitening filter order for pitch estimator
    pub pitchEstimationLPCOrder: opus_int,
    /// Threshold for pitch estimator
    pub pitchEstimationThreshold_Q16: opus_int32,
    /// Cumulative max prediction gain
    pub sum_log_gain_Q7: opus_int32,
    /// Number of survivors in NLSF MSVQ
    pub NLSF_MSVQ_Survivors: opus_int,
    /// Flag for deactivating NLSF interpolation, pitch prediction
    pub first_frame_after_reset: opus_int,
    /// Flag for ensuring codec_control only runs once per packet
    pub controlled_since_last_payload: opus_int,
    /// Warping parameter for warped noise shaping
    pub warping_Q16: opus_int,
    /// Flag to enable constant bitrate
    pub useCBR: opus_int,
    /// Flag to indicate that only buffers are prefilled, no coding
    pub prefillFlag: opus_int,
    /// Pointer to iCDF table for low bits of pitch lag index
    pub pitch_lag_low_bits_iCDF: &'static [opus_uint8],
    /// Pointer to iCDF table for pitch contour index
    pub pitch_contour_iCDF: &'static [opus_uint8],
    /// Pointer to NLSF codebook.
    ///
    /// The C leaves this `NULL` until `silk_init_encoder` selects a codebook;
    /// pointing at the narrowband/medium-band codebook by default keeps the
    /// port free of null dereferences without changing any valid flow.
    pub psNLSF_CB: &'static silk_NLSF_CB_struct,
    pub input_quality_bands_Q15: [opus_int; VAD_N_BANDS as usize],
    pub input_tilt_Q15: opus_int,
    /// Quality setting
    pub SNR_dB_Q7: opus_int,

    pub VAD_flags: [opus_int8; MAX_FRAMES_PER_PACKET as usize],
    pub LBRR_flag: opus_int8,
    pub LBRR_flags: [opus_int; MAX_FRAMES_PER_PACKET as usize],

    pub indices: SideInfoIndices,
    pub pulses: [opus_int8; MAX_FRAME_LENGTH as usize],

    pub arch: opus_int,

    /// Buffer containing input signal
    pub inputBuf: [opus_int16; (MAX_FRAME_LENGTH + 2) as usize],
    pub inputBufIx: opus_int,
    pub nFramesPerPacket: opus_int,
    /// Number of frames analyzed in current packet
    pub nFramesEncoded: opus_int,

    pub nChannelsAPI: opus_int,
    pub nChannelsInternal: opus_int,
    pub channelNb: opus_int,

    /// Parameters For LTP scaling Control
    pub frames_since_onset: opus_int,

    /// Specifically for entropy coding
    pub ec_prevSignalType: opus_int,
    pub ec_prevLagIndex: opus_int16,

    pub resampler_state: silk_resampler_state_struct,

    /// Flag to enable DTX
    pub useDTX: opus_int,
    /// Flag to signal DTX period
    pub inDTX: opus_int,
    /// Counts concecutive nonactive frames, used by DTX
    pub noSpeechCounter: opus_int,

    /// Saves the API setting for query
    pub useInBandFEC: opus_int,
    /// Depends on useInBandFRC, bitrate and packet loss rate
    pub LBRR_enabled: opus_int,
    /// Gains increment for coding LBRR frames
    pub LBRR_GainIncreases: opus_int,
    pub indices_LBRR: [SideInfoIndices; MAX_FRAMES_PER_PACKET as usize],
    pub pulses_LBRR: [[opus_int8; MAX_FRAME_LENGTH as usize]; MAX_FRAMES_PER_PACKET as usize],
}

silk_default!(silk_encoder_state {
    In_HP_State: [0; 2],
    variable_HP_smth1_Q15: 0,
    variable_HP_smth2_Q15: 0,
    sLP: silk_LP_state::default(),
    sVAD: silk_VAD_state::default(),
    sNSQ: silk_nsq_state::default(),
    prev_NLSFq_Q15: [0; MAX_LPC_ORDER as usize],
    speech_activity_Q8: 0,
    allow_bandwidth_switch: 0,
    LBRRprevLastGainIndex: 0,
    prevSignalType: 0,
    prevLag: 0,
    pitch_LPC_win_length: 0,
    max_pitch_lag: 0,
    API_fs_Hz: 0,
    prev_API_fs_Hz: 0,
    maxInternal_fs_Hz: 0,
    minInternal_fs_Hz: 0,
    desiredInternal_fs_Hz: 0,
    fs_kHz: 0,
    nb_subfr: 0,
    frame_length: 0,
    subfr_length: 0,
    ltp_mem_length: 0,
    la_pitch: 0,
    la_shape: 0,
    shapeWinLength: 0,
    TargetRate_bps: 0,
    PacketSize_ms: 0,
    PacketLoss_perc: 0,
    frameCounter: 0,
    Complexity: 0,
    nStatesDelayedDecision: 0,
    useInterpolatedNLSFs: 0,
    shapingLPCOrder: 0,
    predictLPCOrder: 0,
    pitchEstimationComplexity: 0,
    pitchEstimationLPCOrder: 0,
    pitchEstimationThreshold_Q16: 0,
    sum_log_gain_Q7: 0,
    NLSF_MSVQ_Survivors: 0,
    first_frame_after_reset: 0,
    controlled_since_last_payload: 0,
    warping_Q16: 0,
    useCBR: 0,
    prefillFlag: 0,
    pitch_lag_low_bits_iCDF: &[],
    pitch_contour_iCDF: &[],
    psNLSF_CB: &crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB,
    input_quality_bands_Q15: [0; VAD_N_BANDS as usize],
    input_tilt_Q15: 0,
    SNR_dB_Q7: 0,
    VAD_flags: [0; MAX_FRAMES_PER_PACKET as usize],
    LBRR_flag: 0,
    LBRR_flags: [0; MAX_FRAMES_PER_PACKET as usize],
    indices: SideInfoIndices::default(),
    pulses: [0; MAX_FRAME_LENGTH as usize],
    arch: 0,
    inputBuf: [0; (MAX_FRAME_LENGTH + 2) as usize],
    inputBufIx: 0,
    nFramesPerPacket: 0,
    nFramesEncoded: 0,
    nChannelsAPI: 0,
    nChannelsInternal: 0,
    channelNb: 0,
    frames_since_onset: 0,
    ec_prevSignalType: 0,
    ec_prevLagIndex: 0,
    resampler_state: silk_resampler_state_struct::default(),
    useDTX: 0,
    inDTX: 0,
    noSpeechCounter: 0,
    useInBandFEC: 0,
    LBRR_enabled: 0,
    LBRR_GainIncreases: 0,
    indices_LBRR: [SideInfoIndices::default(); MAX_FRAMES_PER_PACKET as usize],
    pulses_LBRR: [[0; MAX_FRAME_LENGTH as usize]; MAX_FRAMES_PER_PACKET as usize],
});

// ---------------------------------------------------------------------------
// OSCE state (ENABLE_OSCE)
// ---------------------------------------------------------------------------

/// `OSCE_FEATURES_MAX_HISTORY` — from `dnn/osce_config.h`.
pub const OSCE_FEATURES_MAX_HISTORY: opus_int = 350;

/// `sizeof(OSCEState)` in the reference configuration (`ENABLE_OSCE` defined,
/// `ENABLE_OSCE_BWE` undefined).
pub const OSCE_STATE_SIZE: usize = 29396;

/// `OSCEFeatureState` — `dnn/osce_structs.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OSCEFeatureState {
    pub numbits_smooth: f32,
    pub pitch_hangover_count: opus_int,
    pub last_lag: opus_int,
    pub last_type: opus_int,
    pub signal_history: [f32; OSCE_FEATURES_MAX_HISTORY as usize],
    pub reset: opus_int,
}

silk_default!(OSCEFeatureState {
    numbits_smooth: 0.0,
    pitch_hangover_count: 0,
    last_lag: 0,
    last_type: 0,
    signal_history: [0.0; OSCE_FEATURES_MAX_HISTORY as usize],
    reset: 0,
});

/// `union { LACEState lace; NoLACEState nolace; } OSCEState` — `dnn/`.
///
/// The network layer types it is built from (`AdaConvState`, `AdaCombState`,
/// `AdaShapeState`, ...) belong to `dnn/nnet.h`, which is out of scope for the
/// port.  The union is kept as an opaque block of exactly the C size (and
/// alignment) so that `silk_decoder_state` keeps the C layout.
#[repr(C, align(4))]
#[derive(Clone, Copy)]
pub struct OSCEState {
    pub _opaque: [u8; OSCE_STATE_SIZE],
}

silk_default!(OSCEState {
    _opaque: [0; OSCE_STATE_SIZE],
});

/// `silk_OSCE_struct` — the `ENABLE_OSCE` member of `silk_decoder_state`.
/// (`silk_OSCE_BWE_struct` is dropped: `ENABLE_OSCE_BWE` is undefined.)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_OSCE_struct {
    pub features: OSCEFeatureState,
    pub state: OSCEState,
    pub method: opus_int,
}

silk_default!(silk_OSCE_struct {
    features: OSCEFeatureState::default(),
    state: OSCEState::default(),
    method: 0,
});

// ---------------------------------------------------------------------------
// Packet loss concealment / comfort noise
// ---------------------------------------------------------------------------

/// `silk_PLC_struct`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_PLC_struct {
    /// Pitch lag to use for voiced concealment
    pub pitchL_Q8: opus_int32,
    /// LTP coefficients to use for voiced concealment
    pub LTPCoef_Q14: [opus_int16; LTP_ORDER as usize],
    pub prevLPC_Q12: [opus_int16; MAX_LPC_ORDER as usize],
    /// Was previous frame lost
    pub last_frame_lost: opus_int,
    /// Seed for unvoiced signal generation
    pub rand_seed: opus_int32,
    /// Scaling of unvoiced random signal
    pub randScale_Q14: opus_int16,
    pub conc_energy: opus_int32,
    pub conc_energy_shift: opus_int,
    pub prevLTP_scale_Q14: opus_int16,
    pub prevGain_Q16: [opus_int32; 2],
    pub fs_kHz: opus_int,
    pub nb_subfr: opus_int,
    pub subfr_length: opus_int,
    pub enable_deep_plc: opus_int,
}

silk_default!(silk_PLC_struct {
    pitchL_Q8: 0,
    LTPCoef_Q14: [0; LTP_ORDER as usize],
    prevLPC_Q12: [0; MAX_LPC_ORDER as usize],
    last_frame_lost: 0,
    rand_seed: 0,
    randScale_Q14: 0,
    conc_energy: 0,
    conc_energy_shift: 0,
    prevLTP_scale_Q14: 0,
    prevGain_Q16: [0; 2],
    fs_kHz: 0,
    nb_subfr: 0,
    subfr_length: 0,
    enable_deep_plc: 0,
});

/// `silk_CNG_struct`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_CNG_struct {
    pub CNG_exc_buf_Q14: [opus_int32; MAX_FRAME_LENGTH as usize],
    pub CNG_smth_NLSF_Q15: [opus_int16; MAX_LPC_ORDER as usize],
    pub CNG_synth_state: [opus_int32; MAX_LPC_ORDER as usize],
    pub CNG_smth_Gain_Q16: opus_int32,
    pub rand_seed: opus_int32,
    pub fs_kHz: opus_int,
}

silk_default!(silk_CNG_struct {
    CNG_exc_buf_Q14: [0; MAX_FRAME_LENGTH as usize],
    CNG_smth_NLSF_Q15: [0; MAX_LPC_ORDER as usize],
    CNG_synth_state: [0; MAX_LPC_ORDER as usize],
    CNG_smth_Gain_Q16: 0,
    rand_seed: 0,
    fs_kHz: 0,
});

// ---------------------------------------------------------------------------
// Decoder state
// ---------------------------------------------------------------------------

/// `silk_decoder_state`
///
/// `#define SILK_DECODER_STATE_RESET_START prev_gain_Q16`: the C resets the
/// decoder state from `prev_gain_Q16` onwards (i.e. everything after the OSCE
/// state); fields up to and including `osce` are preserved.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_decoder_state {
    /// `#ifdef ENABLE_OSCE`
    pub osce: silk_OSCE_struct,
    pub prev_gain_Q16: opus_int32,
    pub exc_Q14: [opus_int32; MAX_FRAME_LENGTH as usize],
    pub sLPC_Q14_buf: [opus_int32; MAX_LPC_ORDER as usize],
    /// Buffer for output signal
    pub outBuf: [opus_int16; (MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH) as usize],
    /// Previous Lag
    pub lagPrev: opus_int,
    /// Previous gain index
    pub LastGainIndex: opus_int8,
    /// Sampling frequency in kHz
    pub fs_kHz: opus_int,
    /// API sample frequency (Hz)
    pub fs_API_hz: opus_int32,
    /// Number of 5 ms subframes in a frame
    pub nb_subfr: opus_int,
    /// Frame length (samples)
    pub frame_length: opus_int,
    /// Subframe length (samples)
    pub subfr_length: opus_int,
    /// Length of LTP memory
    pub ltp_mem_length: opus_int,
    /// LPC order
    pub LPC_order: opus_int,
    /// Used to interpolate LSFs
    pub prevNLSF_Q15: [opus_int16; MAX_LPC_ORDER as usize],
    /// Flag for deactivating NLSF interpolation
    pub first_frame_after_reset: opus_int,
    /// Pointer to iCDF table for low bits of pitch lag index
    pub pitch_lag_low_bits_iCDF: &'static [opus_uint8],
    /// Pointer to iCDF table for pitch contour index
    pub pitch_contour_iCDF: &'static [opus_uint8],

    /// For buffering payload in case of more frames per packet
    pub nFramesDecoded: opus_int,
    pub nFramesPerPacket: opus_int,

    /// Specifically for entropy coding
    pub ec_prevSignalType: opus_int,
    pub ec_prevLagIndex: opus_int16,

    pub VAD_flags: [opus_int; MAX_FRAMES_PER_PACKET as usize],
    pub LBRR_flag: opus_int,
    pub LBRR_flags: [opus_int; MAX_FRAMES_PER_PACKET as usize],

    pub resampler_state: silk_resampler_state_struct,

    /// Pointer to NLSF codebook (see `silk_encoder_state::psNLSF_CB`).
    pub psNLSF_CB: &'static silk_NLSF_CB_struct,

    /// Quantization indices
    pub indices: SideInfoIndices,

    /// CNG state
    pub sCNG: silk_CNG_struct,

    /// Stuff used for PLC
    pub lossCnt: opus_int,
    pub prevSignalType: opus_int,
    pub arch: opus_int,

    pub sPLC: silk_PLC_struct,
}

silk_default!(silk_decoder_state {
    osce: silk_OSCE_struct::default(),
    prev_gain_Q16: 0,
    exc_Q14: [0; MAX_FRAME_LENGTH as usize],
    sLPC_Q14_buf: [0; MAX_LPC_ORDER as usize],
    outBuf: [0; (MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH) as usize],
    lagPrev: 0,
    LastGainIndex: 0,
    fs_kHz: 0,
    fs_API_hz: 0,
    nb_subfr: 0,
    frame_length: 0,
    subfr_length: 0,
    ltp_mem_length: 0,
    LPC_order: 0,
    prevNLSF_Q15: [0; MAX_LPC_ORDER as usize],
    first_frame_after_reset: 0,
    pitch_lag_low_bits_iCDF: &[],
    pitch_contour_iCDF: &[],
    nFramesDecoded: 0,
    nFramesPerPacket: 0,
    ec_prevSignalType: 0,
    ec_prevLagIndex: 0,
    VAD_flags: [0; MAX_FRAMES_PER_PACKET as usize],
    LBRR_flag: 0,
    LBRR_flags: [0; MAX_FRAMES_PER_PACKET as usize],
    resampler_state: silk_resampler_state_struct::default(),
    psNLSF_CB: &crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB,
    indices: SideInfoIndices::default(),
    sCNG: silk_CNG_struct::default(),
    lossCnt: 0,
    prevSignalType: 0,
    arch: 0,
    sPLC: silk_PLC_struct::default(),
});

// ---------------------------------------------------------------------------
// Decoder control
// ---------------------------------------------------------------------------

/// `silk_decoder_control`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct silk_decoder_control {
    /// Prediction and coding parameters
    pub pitchL: [opus_int; MAX_NB_SUBFR as usize],
    pub Gains_Q16: [opus_int32; MAX_NB_SUBFR as usize],
    /// Holds interpolated and final coefficients, 4-byte aligned
    pub PredCoef_Q12: [[opus_int16; MAX_LPC_ORDER as usize]; 2],
    pub LTPCoef_Q14: [opus_int16; (LTP_ORDER * MAX_NB_SUBFR) as usize],
    pub LTP_scale_Q14: opus_int,
}

silk_default!(silk_decoder_control {
    pitchL: [0; MAX_NB_SUBFR as usize],
    Gains_Q16: [0; MAX_NB_SUBFR as usize],
    PredCoef_Q12: [[0; MAX_LPC_ORDER as usize]; 2],
    LTPCoef_Q14: [0; (LTP_ORDER * MAX_NB_SUBFR) as usize],
    LTP_scale_Q14: 0,
});

//! Mirror of `silk/define.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Codec-wide limits.  Also holds the `silk/errors.h` status codes, which
//! `define.h` includes.
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

// ---------------------------------------------------------------------------
// Error messages (`silk/errors.h`, included by `define.h`).  They have no Rust
// module of their own, so they live here.
// ---------------------------------------------------------------------------

/// `SILK_NO_ERROR`
pub const SILK_NO_ERROR: opus_int = 0;
/// `SILK_ENC_INPUT_INVALID_NO_OF_SAMPLES`
pub const SILK_ENC_INPUT_INVALID_NO_OF_SAMPLES: opus_int = -101;
/// `SILK_ENC_FS_NOT_SUPPORTED`
pub const SILK_ENC_FS_NOT_SUPPORTED: opus_int = -102;
/// `SILK_ENC_PACKET_SIZE_NOT_SUPPORTED`
pub const SILK_ENC_PACKET_SIZE_NOT_SUPPORTED: opus_int = -103;
/// `SILK_ENC_PAYLOAD_BUF_TOO_SHORT`
pub const SILK_ENC_PAYLOAD_BUF_TOO_SHORT: opus_int = -104;
/// `SILK_ENC_INVALID_LOSS_RATE`
pub const SILK_ENC_INVALID_LOSS_RATE: opus_int = -105;
/// `SILK_ENC_INVALID_COMPLEXITY_SETTING`
pub const SILK_ENC_INVALID_COMPLEXITY_SETTING: opus_int = -106;
/// `SILK_ENC_INVALID_INBAND_FEC_SETTING`
pub const SILK_ENC_INVALID_INBAND_FEC_SETTING: opus_int = -107;
/// `SILK_ENC_INVALID_DTX_SETTING`
pub const SILK_ENC_INVALID_DTX_SETTING: opus_int = -108;
/// `SILK_ENC_INVALID_CBR_SETTING`
pub const SILK_ENC_INVALID_CBR_SETTING: opus_int = -109;
/// `SILK_ENC_INTERNAL_ERROR`
pub const SILK_ENC_INTERNAL_ERROR: opus_int = -110;
/// `SILK_ENC_INVALID_NUMBER_OF_CHANNELS_ERROR`
pub const SILK_ENC_INVALID_NUMBER_OF_CHANNELS_ERROR: opus_int = -111;
/// `SILK_DEC_INVALID_SAMPLING_FREQUENCY`
pub const SILK_DEC_INVALID_SAMPLING_FREQUENCY: opus_int = -200;
/// `SILK_DEC_PAYLOAD_TOO_LARGE`
pub const SILK_DEC_PAYLOAD_TOO_LARGE: opus_int = -201;
/// `SILK_DEC_PAYLOAD_ERROR`
pub const SILK_DEC_PAYLOAD_ERROR: opus_int = -202;
/// `SILK_DEC_INVALID_FRAME_SIZE`
pub const SILK_DEC_INVALID_FRAME_SIZE: opus_int = -203;

// ---------------------------------------------------------------------------
// Limits.
// ---------------------------------------------------------------------------

/// `ENCODER_NUM_CHANNELS` — max number of encoder channels (1/2).
pub const ENCODER_NUM_CHANNELS: opus_int = 2;
/// `DECODER_NUM_CHANNELS` — number of decoder channels (1/2).
pub const DECODER_NUM_CHANNELS: opus_int = 2;

/// `MAX_FRAMES_PER_PACKET`
pub const MAX_FRAMES_PER_PACKET: opus_int = 3;

/// `MIN_TARGET_RATE_BPS`
pub const MIN_TARGET_RATE_BPS: opus_int = 5000;
/// `MAX_TARGET_RATE_BPS`
pub const MAX_TARGET_RATE_BPS: opus_int = 80000;

/// `LBRR_NB_MIN_RATE_BPS`
pub const LBRR_NB_MIN_RATE_BPS: opus_int = 12000;
/// `LBRR_MB_MIN_RATE_BPS`
pub const LBRR_MB_MIN_RATE_BPS: opus_int = 14000;
/// `LBRR_WB_MIN_RATE_BPS`
pub const LBRR_WB_MIN_RATE_BPS: opus_int = 16000;

/// `NB_SPEECH_FRAMES_BEFORE_DTX` — eq 200 ms.
pub const NB_SPEECH_FRAMES_BEFORE_DTX: opus_int = 10;
/// `MAX_CONSECUTIVE_DTX` — eq 400 ms.
pub const MAX_CONSECUTIVE_DTX: opus_int = 20;
/// `DTX_ACTIVITY_THRESHOLD` (`0.1f`).
pub const DTX_ACTIVITY_THRESHOLD: f32 = 0.1;

/// `VAD_NO_DECISION`
pub const VAD_NO_DECISION: opus_int = -1;
/// `VAD_NO_ACTIVITY`
pub const VAD_NO_ACTIVITY: opus_int = 0;
/// `VAD_ACTIVITY`
pub const VAD_ACTIVITY: opus_int = 1;

/// `MAX_FS_KHZ` — maximum sampling frequency.
pub const MAX_FS_KHZ: opus_int = 16;
/// `MAX_API_FS_KHZ` (`ENABLE_QEXT` is undefined in the reference build).
pub const MAX_API_FS_KHZ: opus_int = 48;

/// `TYPE_NO_VOICE_ACTIVITY`
pub const TYPE_NO_VOICE_ACTIVITY: opus_int = 0;
/// `TYPE_UNVOICED`
pub const TYPE_UNVOICED: opus_int = 1;
/// `TYPE_VOICED`
pub const TYPE_VOICED: opus_int = 2;

/// `CODE_INDEPENDENTLY`
pub const CODE_INDEPENDENTLY: opus_int = 0;
/// `CODE_INDEPENDENTLY_NO_LTP_SCALING`
pub const CODE_INDEPENDENTLY_NO_LTP_SCALING: opus_int = 1;
/// `CODE_CONDITIONALLY`
pub const CODE_CONDITIONALLY: opus_int = 2;

/// `STEREO_QUANT_TAB_SIZE` — settings for stereo processing.
pub const STEREO_QUANT_TAB_SIZE: opus_int = 16;
/// `STEREO_QUANT_SUB_STEPS`
pub const STEREO_QUANT_SUB_STEPS: opus_int = 5;
/// `STEREO_INTERP_LEN_MS` — must be even.
pub const STEREO_INTERP_LEN_MS: opus_int = 8;
/// `STEREO_RATIO_SMOOTH_COEF` — smoothing coef for signal norms and stereo width.
pub const STEREO_RATIO_SMOOTH_COEF: f64 = 0.01;

/// `PITCH_EST_MIN_LAG_MS` — 2 ms -> 500 Hz.
pub const PITCH_EST_MIN_LAG_MS: opus_int = 2;
/// `PITCH_EST_MAX_LAG_MS` — 18 ms -> 56 Hz.
pub const PITCH_EST_MAX_LAG_MS: opus_int = 18;

/// `MAX_NB_SUBFR` — maximum number of subframes.
pub const MAX_NB_SUBFR: opus_int = 4;

/// `LTP_MEM_LENGTH_MS` — number of samples per frame.
pub const LTP_MEM_LENGTH_MS: opus_int = 20;
/// `SUB_FRAME_LENGTH_MS`
pub const SUB_FRAME_LENGTH_MS: opus_int = 5;
/// `MAX_SUB_FRAME_LENGTH` (`SUB_FRAME_LENGTH_MS * MAX_FS_KHZ`).
pub const MAX_SUB_FRAME_LENGTH: opus_int = SUB_FRAME_LENGTH_MS * MAX_FS_KHZ;
/// `MAX_FRAME_LENGTH_MS` (`SUB_FRAME_LENGTH_MS * MAX_NB_SUBFR`).
pub const MAX_FRAME_LENGTH_MS: opus_int = SUB_FRAME_LENGTH_MS * MAX_NB_SUBFR;
/// `MAX_FRAME_LENGTH` (`MAX_FRAME_LENGTH_MS * MAX_FS_KHZ`).
pub const MAX_FRAME_LENGTH: opus_int = MAX_FRAME_LENGTH_MS * MAX_FS_KHZ;

/// `LA_PITCH_MS` — milliseconds of lookahead for pitch analysis.
pub const LA_PITCH_MS: opus_int = 2;
/// `LA_PITCH_MAX`
pub const LA_PITCH_MAX: opus_int = LA_PITCH_MS * MAX_FS_KHZ;

/// `MAX_FIND_PITCH_LPC_ORDER` — order of LPC used in find pitch.
pub const MAX_FIND_PITCH_LPC_ORDER: opus_int = 16;

/// `FIND_PITCH_LPC_WIN_MS`
pub const FIND_PITCH_LPC_WIN_MS: opus_int = 20 + (LA_PITCH_MS << 1);
/// `FIND_PITCH_LPC_WIN_MS_2_SF`
pub const FIND_PITCH_LPC_WIN_MS_2_SF: opus_int = 10 + (LA_PITCH_MS << 1);
/// `FIND_PITCH_LPC_WIN_MAX`
pub const FIND_PITCH_LPC_WIN_MAX: opus_int = FIND_PITCH_LPC_WIN_MS * MAX_FS_KHZ;

/// `LA_SHAPE_MS` — milliseconds of lookahead for noise shape analysis.
pub const LA_SHAPE_MS: opus_int = 5;
/// `LA_SHAPE_MAX`
pub const LA_SHAPE_MAX: opus_int = LA_SHAPE_MS * MAX_FS_KHZ;

/// `SHAPE_LPC_WIN_MAX` — max length of LPC window in noise shape analysis.
pub const SHAPE_LPC_WIN_MAX: opus_int = 15 * MAX_FS_KHZ;

/// `MIN_QGAIN_DB` — dB level of lowest gain quantization level.
pub const MIN_QGAIN_DB: opus_int = 2;
/// `MAX_QGAIN_DB` — dB level of highest gain quantization level.
pub const MAX_QGAIN_DB: opus_int = 88;
/// `N_LEVELS_QGAIN` — number of gain quantization levels.
pub const N_LEVELS_QGAIN: opus_int = 64;
/// `MAX_DELTA_GAIN_QUANT` — max increase in gain quantization index.
pub const MAX_DELTA_GAIN_QUANT: opus_int = 36;
/// `MIN_DELTA_GAIN_QUANT` — max decrease in gain quantization index.
pub const MIN_DELTA_GAIN_QUANT: opus_int = -4;

/// `OFFSET_VL_Q10` — quantization offsets (multiples of 4).
pub const OFFSET_VL_Q10: opus_int = 32;
/// `OFFSET_VH_Q10`
pub const OFFSET_VH_Q10: opus_int = 100;
/// `OFFSET_UVL_Q10`
pub const OFFSET_UVL_Q10: opus_int = 100;
/// `OFFSET_UVH_Q10`
pub const OFFSET_UVH_Q10: opus_int = 240;

/// `QUANT_LEVEL_ADJUST_Q10`
pub const QUANT_LEVEL_ADJUST_Q10: opus_int = 80;

/// `MAX_LPC_STABILIZE_ITERATIONS`
pub const MAX_LPC_STABILIZE_ITERATIONS: opus_int = 16;
/// `MAX_PREDICTION_POWER_GAIN` (`1e4f`).
pub const MAX_PREDICTION_POWER_GAIN: f32 = 1e4;
/// `MAX_PREDICTION_POWER_GAIN_AFTER_RESET` (`1e2f`).
pub const MAX_PREDICTION_POWER_GAIN_AFTER_RESET: f32 = 1e2;

/// `MAX_LPC_ORDER`
pub const MAX_LPC_ORDER: opus_int = 16;
/// `MIN_LPC_ORDER`
pub const MIN_LPC_ORDER: opus_int = 10;

/// `LTP_ORDER` — find pred coef defines.
pub const LTP_ORDER: opus_int = 5;

/// `NB_LTP_CBKS` — LTP quantization settings.
pub const NB_LTP_CBKS: opus_int = 3;

/// `USE_HARM_SHAPING` — flag to use harmonic noise shaping.
pub const USE_HARM_SHAPING: opus_int = 1;

/// `MAX_SHAPE_LPC_ORDER` — max LPC order of noise shaping filters.
pub const MAX_SHAPE_LPC_ORDER: opus_int = 24;

/// `HARM_SHAPE_FIR_TAPS`
pub const HARM_SHAPE_FIR_TAPS: opus_int = 3;

/// `MAX_DEL_DEC_STATES` — max number of delayed decision states.
pub const MAX_DEL_DEC_STATES: opus_int = 4;

/// `LTP_BUF_LENGTH`
pub const LTP_BUF_LENGTH: opus_int = 512;
/// `LTP_MASK`
pub const LTP_MASK: opus_int = LTP_BUF_LENGTH - 1;

/// `DECISION_DELAY`
pub const DECISION_DELAY: opus_int = 40;

/// `SHELL_CODEC_FRAME_LENGTH` — subframes for excitation entropy coding.
pub const SHELL_CODEC_FRAME_LENGTH: opus_int = 16;
/// `LOG2_SHELL_CODEC_FRAME_LENGTH`
pub const LOG2_SHELL_CODEC_FRAME_LENGTH: opus_int = 4;
/// `MAX_NB_SHELL_BLOCKS`
pub const MAX_NB_SHELL_BLOCKS: opus_int = MAX_FRAME_LENGTH / SHELL_CODEC_FRAME_LENGTH;

/// `N_RATE_LEVELS` — rate levels for entropy coding of excitation.
pub const N_RATE_LEVELS: opus_int = 10;

/// `SILK_MAX_PULSES` — max sum of pulses per shell coding frame.
pub const SILK_MAX_PULSES: opus_int = 16;

/// `MAX_MATRIX_SIZE` — max of LPC order and LTP order.
pub const MAX_MATRIX_SIZE: opus_int = MAX_LPC_ORDER;

/// `NSQ_LPC_BUF_LENGTH`
pub const NSQ_LPC_BUF_LENGTH: opus_int = MAX_LPC_ORDER;

// ---------------------------------------------------------------------------
// Voice activity detector.
// ---------------------------------------------------------------------------

/// `VAD_N_BANDS`
pub const VAD_N_BANDS: opus_int = 4;

/// `VAD_INTERNAL_SUBFRAMES_LOG2`
pub const VAD_INTERNAL_SUBFRAMES_LOG2: opus_int = 2;
/// `VAD_INTERNAL_SUBFRAMES`
pub const VAD_INTERNAL_SUBFRAMES: opus_int = 1 << VAD_INTERNAL_SUBFRAMES_LOG2;

/// `VAD_NOISE_LEVEL_SMOOTH_COEF_Q16` — must be < 4096.
pub const VAD_NOISE_LEVEL_SMOOTH_COEF_Q16: opus_int = 1024;
/// `VAD_NOISE_LEVELS_BIAS`
pub const VAD_NOISE_LEVELS_BIAS: opus_int = 50;

/// `VAD_NEGATIVE_OFFSET_Q5` — sigmoid is 0 at -128.
pub const VAD_NEGATIVE_OFFSET_Q5: opus_int = 128;
/// `VAD_SNR_FACTOR_Q16`
pub const VAD_SNR_FACTOR_Q16: opus_int = 45000;

/// `VAD_SNR_SMOOTH_COEF_Q18` — smoothing for SNR measurement.
pub const VAD_SNR_SMOOTH_COEF_Q18: opus_int = 4096;

/// `LSF_COS_TAB_SZ_FIX` — size of the piecewise linear cosine table for LSFs.
pub const LSF_COS_TAB_SZ_FIX: opus_int = 128;

// ---------------------------------------------------------------------------
// NLSF quantizer.
// ---------------------------------------------------------------------------

/// `NLSF_W_Q`
pub const NLSF_W_Q: opus_int = 2;
/// `NLSF_VQ_MAX_VECTORS`
pub const NLSF_VQ_MAX_VECTORS: opus_int = 32;
/// `NLSF_QUANT_MAX_AMPLITUDE`
pub const NLSF_QUANT_MAX_AMPLITUDE: opus_int = 4;
/// `NLSF_QUANT_MAX_AMPLITUDE_EXT`
pub const NLSF_QUANT_MAX_AMPLITUDE_EXT: opus_int = 10;
/// `NLSF_QUANT_LEVEL_ADJ` (`0.1` — a double literal in the C).
pub const NLSF_QUANT_LEVEL_ADJ: f64 = 0.1;
/// `NLSF_QUANT_DEL_DEC_STATES_LOG2`
pub const NLSF_QUANT_DEL_DEC_STATES_LOG2: opus_int = 2;
/// `NLSF_QUANT_DEL_DEC_STATES`
pub const NLSF_QUANT_DEL_DEC_STATES: opus_int = 1 << NLSF_QUANT_DEL_DEC_STATES_LOG2;

/// `TRANSITION_TIME_MS` — 5120 = 64 * FRAME_LENGTH_MS * (TRANSITION_INT_NUM - 1).
pub const TRANSITION_TIME_MS: opus_int = 5120;
/// `TRANSITION_NB` — hardcoded in tables.
pub const TRANSITION_NB: opus_int = 3;
/// `TRANSITION_NA` — hardcoded in tables.
pub const TRANSITION_NA: opus_int = 2;
/// `TRANSITION_INT_NUM` — hardcoded in tables.
pub const TRANSITION_INT_NUM: opus_int = 5;
/// `TRANSITION_FRAMES`
pub const TRANSITION_FRAMES: opus_int = TRANSITION_TIME_MS / MAX_FRAME_LENGTH_MS;
/// `TRANSITION_INT_STEPS`
pub const TRANSITION_INT_STEPS: opus_int = TRANSITION_FRAMES / (TRANSITION_INT_NUM - 1);

/// `BWE_AFTER_LOSS_Q16` — BWE factors to apply after packet loss.
pub const BWE_AFTER_LOSS_Q16: opus_int = 63570;

/// `CNG_BUF_MASK_MAX` — 2^floor(log2(MAX_FRAME_LENGTH))-1.
pub const CNG_BUF_MASK_MAX: opus_int = 255;
/// `CNG_GAIN_SMTH_Q16` — 0.25^(1/4).
pub const CNG_GAIN_SMTH_Q16: opus_int = 4634;
/// `CNG_GAIN_SMTH_THRESHOLD_Q16` — -3 dB.
pub const CNG_GAIN_SMTH_THRESHOLD_Q16: opus_int = 46396;
/// `CNG_NLSF_SMTH_Q16` — 0.25.
pub const CNG_NLSF_SMTH_Q16: opus_int = 16348;

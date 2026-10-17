/* Copyright (c) 2010-2011 Xiph.Org Foundation, Skype Limited
   Written by Jean-Marc Valin and Koen Vos */
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
//! Transliteration of `src/opus_encoder.c` (and, because no other module owns
//! it yet, of `src/analysis.c` + `src/mlp.c` + `src/mlp_data.c`) from libopus
//! 1.6.1 into safe Rust.
//!
//! # Configuration
//!
//! Floating point (`FIXED_POINT`, `FLOAT_APPROX`, `ENABLE_QEXT`, `ENABLE_RES24`
//! undefined); `ENABLE_DRED`, `ENABLE_DEEP_PLC`, `ENABLE_OSCE`,
//! `CUSTOM_MODES`, `ENABLE_HARDENING` defined.  The `#ifdef ENABLE_QEXT` and
//! `#ifdef ENABLE_OSCE_TRAINING_DATA` blocks are dropped (as everywhere else in
//! this port), the `FIXED_POINT` branches are dropped and the float ones kept.
//!
//! `ENABLE_DRED` is defined, so the encoder's DRED control flow is ported
//! faithfully.  The DRED/`lpcnet`/`rdovae` neural pieces themselves are
//! **clearly-marked stubs**: `st->dred_duration` is only ever non-zero after
//! `OPUS_SET_DRED_DURATION`, which the `audiopus`-compatible layer never
//! issues, so every consumer is unreachable for the configurations the
//! acceptance test uses.  `DredEnc::loaded` is set to 1 exactly like the
//! reference (which loads the compiled-in weights), so the observable state
//! matches.
//!
//! # State layout
//!
//! C stores the SILK encoder, the CELT encoder and the DRED encoder inside one
//! allocation (`(char*)st + st->silk_enc_offset`, ...); the port owns them as
//! ordinary fields.  [`OPUS_ENCODER_STRUCT_SIZE`] is the C
//! `sizeof(OpusEncoder)` — the value `opus_encoder_get_size()` is built from —
//! and is validated by `examples/probe_opus_encoder.rs` against the reference.
//!
//! # Analysis
//!
//! `opus_encode_native()` runs `run_analysis()` at complexity >= 7, so the
//! tonality analysis (`src/analysis.c`) and its small DNN (`src/mlp.c` +
//! `src/mlp_data.c`) are part of the bit-exact encoder path.  Since
//! `src/analysis.rs` does not exist in this port (and this module may not edit
//! other files), they are transliterated into the [`analysis`] sub-module at
//! the bottom of this file, keeping the C names.
//!
//! # Range coder buffer
//!
//! C's `ec_enc_init(&enc, data, size)` makes the range coder write straight
//! into the caller's `data` buffer, which is also where the redundant CELT
//! frames are written.  The Rust range coder owns its buffer, so every
//! mutation is bracketed by a copy in/out of `buf[1..]` — but only up to
//! `enc.storage`, because that is the region C's encoder can touch.  That
//! reproduces the shared memory exactly.

use crate::celt::arch::*;
use crate::celt::celt::{
    bitrate_to_bits, bits_to_bitrate, AnalysisInfo, CELT_SET_CHANNELS_REQUEST,
    CELT_SET_END_BAND_REQUEST, CELT_SET_PREDICTION_REQUEST, CELT_SET_SIGNALLING_REQUEST,
    CELT_SET_START_BAND_REQUEST, LEAK_BANDS, OPUS_SET_LFE_REQUEST,
};
use crate::celt::celt_encoder::{
    celt_encode_with_ec, celt_encoder_get_size, celt_encoder_init,
    opus_custom_encoder_ctl as celt_encoder_ctl, opus_custom_encoder_ctl_analysis,
    opus_custom_encoder_ctl_energy_mask, opus_custom_encoder_ctl_silk_info, OpusCustomEncoder,
};
use crate::celt::entcode::{ec_ctx, EC_ILOG};
use crate::celt::entenc::{ec_enc_bit_logp, ec_enc_done, ec_enc_init, ec_enc_shrink, ec_enc_uint};
use crate::celt::kernels::celt_inner_prod;
use crate::celt::mathops::{celt_exp2, celt_maxabs_res, celt_sqrt, fast_atan2f, frac_div32};
use crate::celt::modes::CELTMode;
use crate::celt::quant_bands::DB_SHIFT;
use crate::ffi::*;
use crate::opus::packet::{MODE_CELT_ONLY, MODE_HYBRID, MODE_SILK_ONLY};
use crate::opus::repacketizer::{opus_packet_pad, opus_packet_pad_impl, OpusRepacketizer};
use crate::silk::control::silk_EncControlStruct;
use crate::silk::define::*;
use crate::silk::enc_api::{silk_Encode, silk_Get_Encoder_Size, silk_InitEncoder};
use crate::silk::float::main_flp::silk_encoder;
use crate::silk::init_decoder::opus_select_arch;
use crate::silk::lin2log::silk_lin2log;
use crate::silk::log2lin::silk_log2lin;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;
/* `types` and `silk::typedef` both define these aliases; an explicit import
   resolves the glob ambiguity (the aliases are identical). */
use crate::types::{opus_int16, opus_int32, opus_uint32};

/* ------------------------------------------------------------------------- *
 * Constants
 * ------------------------------------------------------------------------- */

/// `sizeof(struct OpusEncoder)` in this configuration (float build, DRED +
/// analysis + the 960-entry `delay_buffer`).  Measured from the reference; it
/// is what `opus_encoder_get_size()` is built from.
pub const OPUS_ENCODER_STRUCT_SIZE: i32 = 95592;

/// `silk_Get_Encoder_Size(&size, 1)` in the reference.
pub const SILK_ENCODER_SIZE_1: i32 = 10272;
/// `silk_Get_Encoder_Size(&size, 2)` in the reference.
pub const SILK_ENCODER_SIZE_2: i32 = 20456;

/// `#define MAX_ENCODER_BUFFER 480` (`ENABLE_QEXT` undefined).
const MAX_ENCODER_BUFFER: i32 = 480;

/// `#define PSEUDO_SNR_THRESHOLD 316.23f`
const PSEUDO_SNR_THRESHOLD: f32 = 316.23;

/// `OPUS_SET_VOICE_RATIO_REQUEST` (`src/opus_private.h`) — not in `ffi.rs`.
const OPUS_SET_VOICE_RATIO_REQUEST: i32 = 11018;
/// `OPUS_GET_VOICE_RATIO_REQUEST` (`src/opus_private.h`).
pub(crate) const OPUS_GET_VOICE_RATIO_REQUEST: i32 = 11019;
/// `OPUS_SET_FORCE_MODE_REQUEST` (`src/opus_private.h`).
pub(crate) const OPUS_SET_FORCE_MODE_REQUEST: i32 = 11002;

/// `VARIABLE_HP_MIN_CUTOFF_HZ` (`silk/tuning_parameters.h`).
const VARIABLE_HP_MIN_CUTOFF_HZ: opus_int = 60;
/// `VARIABLE_HP_SMTH_COEF2` (`silk/tuning_parameters.h`).
const VARIABLE_HP_SMTH_COEF2: f64 = 0.015;

/* `dnn/dred_config.h` */
const DRED_EXTENSION_ID: i32 = 126;
const DRED_EXPERIMENTAL_BYTES: i32 = 2;
const DRED_MIN_BYTES: i32 = 8;
const DRED_MAX_DATA_SIZE: i32 = 1000;
const DRED_MAX_LATENTS: i32 = 26;
const DRED_NUM_REDUNDANCY_FRAMES: i32 = 2 * DRED_MAX_LATENTS;
const DRED_MAX_FRAMES: i32 = 4 * DRED_MAX_LATENTS;
/// `DRED_SILK_ENCODER_DELAY (79+12-80)`
const DRED_SILK_ENCODER_DELAY: i32 = 79 + 12 - 80;

/* `src/opus_encoder.c` transition tables. */
static mono_voice_bandwidth_thresholds: [opus_int32; 8] = [9000, 700, 9000, 700, 13500, 1000, 14000, 2000];
static mono_music_bandwidth_thresholds: [opus_int32; 8] = [9000, 700, 9000, 700, 11000, 1000, 12000, 2000];
static stereo_voice_bandwidth_thresholds: [opus_int32; 8] = [9000, 700, 9000, 700, 13500, 1000, 14000, 2000];
static stereo_music_bandwidth_thresholds: [opus_int32; 8] = [9000, 700, 9000, 700, 11000, 1000, 12000, 2000];
static stereo_voice_threshold: opus_int32 = 19000;
static stereo_music_threshold: opus_int32 = 17000;
static mode_thresholds: [[opus_int32; 2]; 2] = [[64000, 10000], [44000, 10000]];
static fec_thresholds: [opus_int32; 10] = [12000, 1000, 14000, 1000, 16000, 1000, 20000, 1000, 22000, 1000];

/* ------------------------------------------------------------------------- *
 * Small helpers
 * ------------------------------------------------------------------------- */

/// `MAXG(a,b)` (`celt/arch.h`: `MAX32`).
#[inline(always)]
fn MAXG(a: f32, b: f32) -> f32 {
    MAX32(a, b)
}
/// `MING(a,b)` (`celt/arch.h`: `MIN32`).
#[inline(always)]
fn MING(a: f32, b: f32) -> f32 {
    MIN32(a, b)
}
/// `GCONST(x)` in a float build is just `x`.
#[inline(always)]
fn GCONST(x: f32) -> f32 {
    x
}

/// `align()` (`src/opus_private.h`): rounds up to the alignment of a
/// `union { void*; opus_int32; opus_val32; }`, which is 8 on x86-64.
#[inline(always)]
fn align(i: i32) -> i32 {
    ((i + 7) / 8) * 8
}

/* ------------------------------------------------------------------------- *
 * struct StereoWidthState / struct OpusEncoder
 * ------------------------------------------------------------------------- */

/// `StereoWidthState` (`src/opus_encoder.c:70`).
#[derive(Clone, Copy, Default)]
pub struct StereoWidthState {
    pub XX: opus_val32,
    pub XY: opus_val32,
    pub YY: opus_val32,
    pub smoothed_width: opus_val16,
    pub max_follower: opus_val16,
}

/// `DREDEnc` (`dnn/dred_encoder.h`), reduced to the members the encoder reads.
///
/// **DNN stub**: the RDOVAE/LPCNet weights are not ported.  `loaded` is 1 to
/// match the reference (which loads the compiled-in weights); the only
/// consumers are gated on `st->dred_duration > 0`, which requires
/// `OPUS_SET_DRED_DURATION` — never issued by the `audiopus`-compatible API.
#[derive(Clone)]
pub struct DredEnc {
    /// `int loaded`
    pub loaded: i32,
    /// `opus_int32 Fs`
    pub Fs: i32,
    /// `int channels`
    pub channels: i32,
    /// `int input_buffer_fill`
    pub input_buffer_fill: i32,
    /// `int dred_offset`
    pub dred_offset: i32,
    /// `int latent_offset`
    pub latent_offset: i32,
    /// `int last_extra_dred_offset`
    pub last_extra_dred_offset: i32,
    /// `int latents_buffer_fill`
    pub latents_buffer_fill: i32,
}

impl Default for DredEnc {
    fn default() -> Self {
        DredEnc {
            loaded: 0,
            Fs: 0,
            channels: 0,
            input_buffer_fill: 0,
            dred_offset: 0,
            latent_offset: 0,
            last_extra_dred_offset: 0,
            latents_buffer_fill: 0,
        }
    }
}

/// `struct OpusEncoder` from `src/opus_encoder.c`.
///
/// The three `*_enc_offset` members of the C struct have no counterpart: the
/// SILK, CELT and DRED encoders are owned fields here.  Everything from
/// `stream_channels` (`OPUS_ENCODER_RESET_START`) to `delay_buffer` is the C
/// "reset region", cleared by `OPUS_RESET_STATE`.
pub struct OpusEncoder {
    /// `silk_EncControlStruct silk_mode`
    pub silk_mode: silk_EncControlStruct,
    /// `DREDEnc dred_encoder`
    pub dred_encoder: DredEnc,
    /// `int application`
    pub application: i32,
    /// `int channels`
    pub channels: i32,
    /// `int delay_compensation`
    pub delay_compensation: i32,
    /// `int force_channels`
    pub force_channels: i32,
    /// `int signal_type`
    pub signal_type: i32,
    /// `int user_bandwidth`
    pub user_bandwidth: i32,
    /// `int max_bandwidth`
    pub max_bandwidth: i32,
    /// `int user_forced_mode`
    pub user_forced_mode: i32,
    /// `int voice_ratio`
    pub voice_ratio: i32,
    /// `opus_int32 Fs`
    pub Fs: i32,
    /// `int use_vbr`
    pub use_vbr: i32,
    /// `int vbr_constraint`
    pub vbr_constraint: i32,
    /// `int variable_duration`
    pub variable_duration: i32,
    /// `opus_int32 bitrate_bps`
    pub bitrate_bps: i32,
    /// `opus_int32 user_bitrate_bps`
    pub user_bitrate_bps: i32,
    /// `int lsb_depth`
    pub lsb_depth: i32,
    /// `int encoder_buffer`
    pub encoder_buffer: i32,
    /// `int lfe`
    pub lfe: i32,
    /// `int arch`
    pub arch: i32,
    /// `int use_dtx`
    pub use_dtx: i32,
    /// `int fec_config`
    pub fec_config: i32,
    /// `TonalityAnalysisState analysis`
    pub analysis: analysis::TonalityAnalysisState,
    /* Everything beyond this point gets cleared on a reset. */
    /// `int stream_channels`
    pub stream_channels: i32,
    /// `opus_int16 hybrid_stereo_width_Q14`
    pub hybrid_stereo_width_Q14: i16,
    /// `opus_int32 variable_HP_smth2_Q15`
    pub variable_HP_smth2_Q15: i32,
    /// `opus_val16 prev_HB_gain`
    pub prev_HB_gain: opus_val16,
    /// `opus_val32 hp_mem[4]`
    pub hp_mem: [opus_val32; 4],
    /// `int mode`
    pub mode: i32,
    /// `int prev_mode`
    pub prev_mode: i32,
    /// `int prev_channels`
    pub prev_channels: i32,
    /// `int prev_framesize`
    pub prev_framesize: i32,
    /// `int bandwidth`
    pub bandwidth: i32,
    /// `int auto_bandwidth`
    pub auto_bandwidth: i32,
    /// `int silk_bw_switch`
    pub silk_bw_switch: i32,
    /// `int first`
    pub first: i32,
    /// `celt_glog *energy_masking` (C borrows the caller's array; the port owns
    /// a copy).
    pub energy_masking: Option<Vec<celt_glog>>,
    /// `StereoWidthState width_mem`
    pub width_mem: StereoWidthState,
    /// `int detected_bandwidth`
    pub detected_bandwidth: i32,
    /// `int nb_no_activity_ms_Q1`
    pub nb_no_activity_ms_Q1: i32,
    /// `opus_val32 peak_signal_energy`
    pub peak_signal_energy: opus_val32,
    /// `int dred_duration`
    pub dred_duration: i32,
    /// `int dred_q0`
    pub dred_q0: i32,
    /// `int dred_dQ`
    pub dred_dQ: i32,
    /// `int dred_qmax`
    pub dred_qmax: i32,
    /// `int dred_target_chunks`
    pub dred_target_chunks: i32,
    /// `unsigned char activity_mem[DRED_MAX_FRAMES*4]`
    pub activity_mem: [u8; (DRED_MAX_FRAMES * 4) as usize],
    /// `int nonfinal_frame`
    pub nonfinal_frame: i32,
    /// `opus_uint32 rangeFinal`
    pub rangeFinal: opus_uint32,
    /// `opus_res delay_buffer[MAX_ENCODER_BUFFER*2]`
    pub delay_buffer: [opus_res; (MAX_ENCODER_BUFFER * 2) as usize],
    /* The encoder states C places in the same allocation. */
    /// The `silk_encoder` at `silk_enc_offset`.
    pub silk_enc: silk_encoder,
    /// The `CELTEncoder` at `celt_enc_offset`.
    pub celt_enc: OpusCustomEncoder,
}

impl Default for OpusEncoder {
    fn default() -> Self {
        OpusEncoder {
            silk_mode: silk_EncControlStruct::default(),
            dred_encoder: DredEnc::default(),
            application: 0,
            channels: 0,
            delay_compensation: 0,
            force_channels: 0,
            signal_type: 0,
            user_bandwidth: 0,
            max_bandwidth: 0,
            user_forced_mode: 0,
            voice_ratio: 0,
            Fs: 0,
            use_vbr: 0,
            vbr_constraint: 0,
            variable_duration: 0,
            bitrate_bps: 0,
            user_bitrate_bps: 0,
            lsb_depth: 0,
            encoder_buffer: 0,
            lfe: 0,
            arch: 0,
            use_dtx: 0,
            fec_config: 0,
            analysis: analysis::TonalityAnalysisState::default(),
            stream_channels: 0,
            hybrid_stereo_width_Q14: 0,
            variable_HP_smth2_Q15: 0,
            prev_HB_gain: 0.0,
            hp_mem: [0.0; 4],
            mode: 0,
            prev_mode: 0,
            prev_channels: 0,
            prev_framesize: 0,
            bandwidth: 0,
            auto_bandwidth: 0,
            silk_bw_switch: 0,
            first: 0,
            energy_masking: None,
            width_mem: StereoWidthState::default(),
            detected_bandwidth: 0,
            nb_no_activity_ms_Q1: 0,
            peak_signal_energy: 0.0,
            dred_duration: 0,
            dred_q0: 0,
            dred_dQ: 0,
            dred_qmax: 0,
            dred_target_chunks: 0,
            activity_mem: [0; (DRED_MAX_FRAMES * 4) as usize],
            nonfinal_frame: 0,
            rangeFinal: 0,
            delay_buffer: [0.0; (MAX_ENCODER_BUFFER * 2) as usize],
            silk_enc: silk_encoder::default(),
            celt_enc: OpusCustomEncoder::zeroed(),
        }
    }
}

/* ------------------------------------------------------------------------- *
 * gen_toc
 * ------------------------------------------------------------------------- */

/// `gen_toc()` (`src/opus_encoder.c:305`).
fn gen_toc(mode: i32, mut framerate: i32, bandwidth: i32, channels: i32) -> u8 {
    let mut period = 0;
    let mut toc: u8;
    while framerate < 400 {
        framerate <<= 1;
        period += 1;
    }
    if mode == MODE_SILK_ONLY {
        toc = ((bandwidth - OPUS_BANDWIDTH_NARROWBAND) << 5) as u8;
        toc |= ((period - 2) << 3) as u8;
    } else if mode == MODE_CELT_ONLY {
        let mut tmp = bandwidth - OPUS_BANDWIDTH_MEDIUMBAND;
        if tmp < 0 {
            tmp = 0;
        }
        toc = 0x80;
        toc |= (tmp << 5) as u8;
        toc |= (period << 3) as u8;
    } else {
        /* Hybrid */
        toc = 0x60;
        toc |= ((bandwidth - OPUS_BANDWIDTH_SUPERWIDEBAND) << 4) as u8;
        toc |= ((period - 2) << 3) as u8;
    }
    toc |= ((channels == 2) as u8) << 2;
    toc
}

/* ------------------------------------------------------------------------- *
 * silk_biquad_res / hp_cutoff / dc_reject (the float branches)
 * ------------------------------------------------------------------------- */

/// `silk_biquad_res()` (`src/opus_encoder.c:385`, `#else` float branch).
fn silk_biquad_res(
    in_: &[opus_res],        /* I:    Input signal                   */
    B_Q28: &[opus_int32; 3], /* I:    MA coefficients [3]            */
    A_Q28: &[opus_int32; 2], /* I:    AR coefficients [2]            */
    S: &mut [opus_val32],    /* I/O:  State vector [2]               */
    out: &mut [opus_res],    /* O:    Output signal                  */
    len: opus_int32,         /* I:    Signal length (must be even)   */
    stride: i32,
) {
    let mut A = [0.0f32; 2];
    let mut B = [0.0f32; 3];

    A[0] = (A_Q28[0] as f32) * (1.0f32 / ((1i32 << 28) as f32));
    A[1] = (A_Q28[1] as f32) * (1.0f32 / ((1i32 << 28) as f32));
    B[0] = (B_Q28[0] as f32) * (1.0f32 / ((1i32 << 28) as f32));
    B[1] = (B_Q28[1] as f32) * (1.0f32 / ((1i32 << 28) as f32));
    B[2] = (B_Q28[2] as f32) * (1.0f32 / ((1i32 << 28) as f32));

    for k in 0..len {
        let inval = in_[(k * stride) as usize];
        let vout = S[0] + B[0] * inval;

        S[0] = S[1] - vout * A[0] + B[1] * inval;

        S[1] = -vout * A[1] + B[2] * inval + VERY_SMALL;

        out[(k * stride) as usize] = vout;
    }
}

/// `hp_cutoff()` (`src/opus_encoder.c:437`).
fn hp_cutoff(
    in_: &[opus_res],
    cutoff_Hz: opus_int32,
    out: &mut [opus_res],
    hp_mem: &mut [opus_val32],
    len: i32,
    channels: i32,
    Fs: opus_int32,
    arch: i32,
) {
    let mut B_Q28 = [0 as opus_int32; 3];
    let mut A_Q28 = [0 as opus_int32; 2];
    let Fc_Q19;
    let r_Q28;
    let r_Q22;
    let _ = arch;

    debug_assert!(cutoff_Hz <= silk_int32_MAX / SILK_FIX_CONST(1.5 * 3.14159 / 1000.0, 19));
    Fc_Q19 = silk_DIV32_16(
        silk_SMULBB(SILK_FIX_CONST(1.5 * 3.14159 / 1000.0, 19), cutoff_Hz),
        Fs / 1000,
    );
    debug_assert!(Fc_Q19 > 0 && Fc_Q19 < 32768);

    r_Q28 = SILK_FIX_CONST(1.0, 28) - silk_MUL(SILK_FIX_CONST(0.92, 9), Fc_Q19);

    B_Q28[0] = r_Q28;
    B_Q28[1] = silk_LSHIFT(-r_Q28, 1);
    B_Q28[2] = r_Q28;

    r_Q22 = silk_RSHIFT(r_Q28, 6);
    A_Q28[0] = silk_SMULWW(r_Q22, silk_SMULWW(Fc_Q19, Fc_Q19) - SILK_FIX_CONST(2.0, 22));
    A_Q28[1] = silk_SMULWW(r_Q22, r_Q22);

    silk_biquad_res(in_, &B_Q28, &A_Q28, &mut hp_mem[0..2], out, len, channels);
    if channels == 2 {
        silk_biquad_res(
            &in_[1..],
            &B_Q28,
            &A_Q28,
            &mut hp_mem[2..4],
            &mut out[1..],
            len,
            channels,
        );
    }
}

/// `dc_reject()` (`src/opus_encoder.c:500`, `#else` float branch).
fn dc_reject(
    in_: &[opus_val16],
    cutoff_Hz: opus_int32,
    out: &mut [opus_val16],
    hp_mem: &mut [opus_val32],
    len: i32,
    channels: i32,
    Fs: opus_int32,
) {
    let coef = 6.3f32 * (cutoff_Hz as f32) / (Fs as f32);
    let coef2 = 1.0 - coef;
    if channels == 2 {
        let mut m0 = hp_mem[0];
        let mut m2 = hp_mem[2];
        for i in 0..len {
            let x0 = in_[(2 * i) as usize];
            let x1 = in_[(2 * i + 1) as usize];
            let out0 = x0 - m0;
            let out1 = x1 - m2;
            m0 = coef * x0 + VERY_SMALL + coef2 * m0;
            m2 = coef * x1 + VERY_SMALL + coef2 * m2;
            out[(2 * i) as usize] = out0;
            out[(2 * i + 1) as usize] = out1;
        }
        hp_mem[0] = m0;
        hp_mem[2] = m2;
    } else {
        let mut m0 = hp_mem[0];
        for i in 0..len {
            let x = in_[i as usize];
            let y = x - m0;
            m0 = coef * x + VERY_SMALL + coef2 * m0;
            out[i as usize] = y;
        }
        hp_mem[0] = m0;
    }
}

/// `stereo_fade()` (`src/opus_encoder.c:545`).
///
/// The C passes the same buffer as `in` and `out`; the Rust port does the fade
/// in place (the arithmetic order is identical because `stereo_fade` only ever
/// reads a sample before writing it).
fn stereo_fade(
    buf: &mut [opus_res],
    mut g1: opus_val16,
    mut g2: opus_val16,
    overlap48: i32,
    frame_size: i32,
    channels: i32,
    window: &[celt_coef],
    Fs: opus_int32,
) {
    let inc = IMAX(1, 48000 / Fs);
    let overlap = overlap48 / inc;
    g1 = Q15ONE - g1;
    g2 = Q15ONE - g2;
    let mut i = 0;
    while i < overlap {
        let w = COEF2VAL16(window[(i * inc) as usize]);
        let w = MULT16_16_Q15(w, w);
        let g = SHR32(MAC16_16(MULT16_16(w, g2), Q15ONE - w, g1), 15);
        let mut diff = HALF32(buf[(i * channels) as usize] - buf[(i * channels + 1) as usize]);
        diff = MULT16_RES_Q15(g, diff);
        buf[(i * channels) as usize] = buf[(i * channels) as usize] - diff;
        buf[(i * channels + 1) as usize] = buf[(i * channels + 1) as usize] + diff;
        i += 1;
    }
    while i < frame_size {
        let mut diff = HALF32(buf[(i * channels) as usize] - buf[(i * channels + 1) as usize]);
        diff = MULT16_RES_Q15(g2, diff);
        buf[(i * channels) as usize] = buf[(i * channels) as usize] - diff;
        buf[(i * channels + 1) as usize] = buf[(i * channels + 1) as usize] + diff;
        i += 1;
    }
}

/// `gain_fade()` (`src/opus_encoder.c:585`).
fn gain_fade(
    buf: &mut [opus_res],
    g1: opus_val16,
    g2: opus_val16,
    overlap48: i32,
    frame_size: i32,
    channels: i32,
    window: &[celt_coef],
    Fs: opus_int32,
) {
    let inc = IMAX(1, 48000 / Fs);
    let overlap = overlap48 / inc;
    let mut i = 0;
    if channels == 1 {
        while i < overlap {
            let w = COEF2VAL16(window[(i * inc) as usize]);
            let w = MULT16_16_Q15(w, w);
            let g = SHR32(MAC16_16(MULT16_16(w, g2), Q15ONE - w, g1), 15);
            buf[i as usize] = MULT16_RES_Q15(g, buf[i as usize]);
            i += 1;
        }
    } else {
        while i < overlap {
            let w = COEF2VAL16(window[(i * inc) as usize]);
            let w = MULT16_16_Q15(w, w);
            let g = SHR32(MAC16_16(MULT16_16(w, g2), Q15ONE - w, g1), 15);
            buf[(i * 2) as usize] = MULT16_RES_Q15(g, buf[(i * 2) as usize]);
            buf[(i * 2 + 1) as usize] = MULT16_RES_Q15(g, buf[(i * 2 + 1) as usize]);
            i += 1;
        }
    }
    let mut c = 0;
    loop {
        let mut i = overlap;
        while i < frame_size {
            buf[(i * channels + c) as usize] = MULT16_RES_Q15(g2, buf[(i * channels + c) as usize]);
            i += 1;
        }
        c += 1;
        if c >= channels {
            break;
        }
    }
}

/* ------------------------------------------------------------------------- *
 * DRED glue (stubs — see the module comment)
 * ------------------------------------------------------------------------- */

/// `compute_quantizer()` (`dnn/dred_coding.c:39`).
pub fn compute_quantizer(q0: i32, dQ: i32, qmax: i32, i: i32) -> i32 {
    static dQ_table: [i32; 8] = [0, 2, 3, 4, 6, 8, 12, 16];
    let quant = q0 + (dQ_table[dQ as usize] * i + 8) / 16;
    if quant > qmax {
        qmax
    } else {
        quant
    }
}

/// `estimate_dred_bitrate()` (`src/opus_encoder.c:690`).
fn estimate_dred_bitrate(
    q0: i32,
    dQ: i32,
    qmax: i32,
    duration: i32,
    target_bits: opus_int32,
    target_chunks: Option<&mut i32>,
) -> i32 {
    static dred_bits_table: [f32; 16] = [
        73.2, 68.1, 62.5, 57.0, 51.5, 45.7, 39.9, 32.4, 26.4, 20.4, 16.3, 13.0, 9.3, 8.2, 7.2,
        6.4,
    ];
    /* Signaling DRED costs 3 bytes. */
    let mut bits: f32 = 8.0 * ((3 + DRED_EXPERIMENTAL_BYTES) as f32);
    /* Approximation for the size of the IS. */
    bits += 50.0 + dred_bits_table[q0 as usize];
    let dred_chunks = IMIN((duration + 5) / 4, DRED_NUM_REDUNDANCY_FRAMES / 2);
    let mut tc = 0;
    for i in 0..dred_chunks {
        let q = compute_quantizer(q0, dQ, qmax, i);
        bits += dred_bits_table[q as usize];
        /* C compares `float bits` with `opus_int32 target_bits`. */
        if bits < target_bits as f32 {
            tc = i + 1;
        }
    }
    if let Some(t) = target_chunks {
        *t = tc;
    }
    (0.5f32 + bits).floor() as i32
}

/// `compute_dred_bitrate()` (`src/opus_encoder.c:713`).
fn compute_dred_bitrate(st: &mut OpusEncoder, bitrate_bps: opus_int32, frame_size: i32) -> opus_int32 {
    let dred_frac;
    let bitrate_offset;
    let target_dred_bitrate;
    let mut target_chunks: i32 = 0;
    let max_dred_bits;
    let q0;
    let dQ;
    let qmax;
    if st.silk_mode.useInBandFEC != 0 {
        dred_frac = MIN16(0.7f32, 3.0 * (st.silk_mode.packetLossPercentage as f32) / 100.0);
        bitrate_offset = 20000;
    } else {
        if st.silk_mode.packetLossPercentage > 5 {
            dred_frac = MIN16(
                0.8f32,
                0.55f32 + (st.silk_mode.packetLossPercentage as f32) / 100.0,
            );
        } else {
            dred_frac = 12.0 * (st.silk_mode.packetLossPercentage as f32) / 100.0;
        }
        bitrate_offset = 12000;
    }
    /* Account for the fact that longer packets require less redundancy. */
    let dred_frac =
        dred_frac / (dred_frac + (1.0 - dred_frac) * ((frame_size as f32) * 50.0) / (st.Fs as f32));
    /* Approximate fit based on a few experiments. Could probably be improved. */
    q0 = IMIN(15, IMAX(4, 51 - 3 * EC_ILOG(IMAX(1, bitrate_bps - bitrate_offset) as u32)));
    dQ = if bitrate_bps - bitrate_offset > 36000 { 3 } else { 5 };
    qmax = 15;
    target_dred_bitrate = IMAX(0, (dred_frac * ((bitrate_bps - bitrate_offset) as f32)) as i32);
    if st.dred_duration > 0 {
        let target_bits = bitrate_to_bits(target_dred_bitrate, st.Fs, frame_size);
        max_dred_bits = estimate_dred_bitrate(
            q0,
            dQ,
            qmax,
            st.dred_duration,
            target_bits,
            Some(&mut target_chunks),
        );
    } else {
        max_dred_bits = 0;
        target_chunks = 0;
    }
    let dred_bitrate = IMIN(
        target_dred_bitrate,
        bits_to_bitrate(max_dred_bits, st.Fs, frame_size),
    );
    /* If we can't afford enough bits, don't bother with DRED at all. */
    if target_chunks < 2 {
        st.dred_q0 = q0;
        st.dred_dQ = dQ;
        st.dred_qmax = qmax;
        st.dred_target_chunks = target_chunks;
        return 0;
    }
    st.dred_q0 = q0;
    st.dred_dQ = dQ;
    st.dred_qmax = qmax;
    st.dred_target_chunks = target_chunks;
    dred_bitrate
}

/// `dred_encoder_init()` (`dnn/dred_encoder.c:79`).  **DNN stub**: sets the
/// scalar state and marks the (unported) model as loaded, exactly as the
/// reference does with its compiled-in weights.
fn dred_encoder_init(enc: &mut DredEnc, Fs: opus_int32, channels: i32) {
    enc.Fs = Fs;
    enc.channels = channels;
    enc.loaded = 1;
    dred_encoder_reset(enc);
}

/// `dred_encoder_reset()` (`dnn/dred_encoder.c:69`).  **DNN stub**: clears the
/// scalar reset region and sets `input_buffer_fill`; the LPCNet/RDOVAE resets
/// only touch the unported network state.
fn dred_encoder_reset(enc: &mut DredEnc) {
    enc.dred_offset = 0;
    enc.latent_offset = 0;
    enc.last_extra_dred_offset = 0;
    enc.latents_buffer_fill = 0;
    enc.input_buffer_fill = DRED_SILK_ENCODER_DELAY;
}

/// `dred_compute_latents()` (`dnn/dred_encoder.c:211`).  **DNN stub**:
/// unreachable while `st->dred_duration == 0`.
fn dred_compute_latents(
    enc: &mut DredEnc,
    _pcm: &[f32],
    _frame_size: i32,
    _extra_delay: i32,
    _arch: i32,
) {
    enc.latents_buffer_fill = 0;
}

/// `dred_encode_silk_frame()` (`dnn/dred_encoder.c:279`).  **DNN stub**:
/// reports "nothing encoded", which is what the caller observes when DRED is
/// off.
#[allow(clippy::too_many_arguments)]
fn dred_encode_silk_frame(
    _enc: &mut DredEnc,
    _buf: &mut [u8],
    _max_chunks: i32,
    _max_bytes: i32,
    _q0: i32,
    _dQ: i32,
    _qmax: i32,
    _activity_mem: &[u8],
    _arch: i32,
) -> i32 {
    0
}

/* ------------------------------------------------------------------------- *
 * user_bitrate_to_bitrate
 * ------------------------------------------------------------------------- */

/// `user_bitrate_to_bitrate()` (`src/opus_encoder.c:756`).
fn user_bitrate_to_bitrate(st: &OpusEncoder, mut frame_size: i32, max_data_bytes: i32) -> opus_int32 {
    if frame_size == 0 {
        frame_size = st.Fs / 400;
    }
    let max_bitrate = bits_to_bitrate(max_data_bytes * 8, st.Fs, frame_size);
    let user_bitrate;
    if st.user_bitrate_bps == OPUS_AUTO {
        user_bitrate = 60 * st.Fs / frame_size + st.Fs * st.channels;
    } else if st.user_bitrate_bps == OPUS_BITRATE_MAX {
        user_bitrate = 1500000;
    } else {
        user_bitrate = st.user_bitrate_bps;
    }
    IMIN(user_bitrate, max_bitrate)
}

/* ------------------------------------------------------------------------- *
 * frame_size_select / compute_stereo_width
 * ------------------------------------------------------------------------- */

/// `frame_size_select()` (`src/opus_encoder.c:841`).
pub fn frame_size_select(
    application: i32,
    frame_size: opus_int32,
    variable_duration: i32,
    Fs: opus_int32,
) -> i32 {
    let new_size;
    if frame_size < Fs / 400 {
        return -1;
    }
    if variable_duration == OPUS_FRAMESIZE_ARG {
        new_size = frame_size;
    } else if variable_duration >= OPUS_FRAMESIZE_2_5_MS && variable_duration <= OPUS_FRAMESIZE_120_MS
    {
        if variable_duration <= OPUS_FRAMESIZE_40_MS {
            new_size = (Fs / 400) << (variable_duration - OPUS_FRAMESIZE_2_5_MS);
        } else {
            new_size = (variable_duration - OPUS_FRAMESIZE_2_5_MS - 2) * Fs / 50;
        }
    } else {
        return -1;
    }
    if new_size > frame_size {
        return -1;
    }
    if 400 * new_size != Fs
        && 200 * new_size != Fs
        && 100 * new_size != Fs
        && 50 * new_size != Fs
        && 25 * new_size != Fs
        && 50 * new_size != 3 * Fs
        && 50 * new_size != 4 * Fs
        && 50 * new_size != 5 * Fs
        && 50 * new_size != 6 * Fs
    {
        return -1;
    }
    if application == OPUS_APPLICATION_RESTRICTED_SILK && new_size < Fs / 100 {
        return -1;
    }
    new_size
}

/// `compute_stereo_width()` (`src/opus_encoder.c:880`).
pub fn compute_stereo_width(
    pcm: &[opus_res],
    frame_size: i32,
    Fs: opus_int32,
    mem: &mut StereoWidthState,
) -> opus_val16 {
    let mut xx: opus_val32;
    let mut xy: opus_val32;
    let mut yy: opus_val32;
    let sqrt_xx;
    let sqrt_yy;
    let qrrt_xx;
    let qrrt_yy;
    let frame_rate = Fs / frame_size;
    let short_alpha = MULT16_16(25.0, Q15ONE) / (IMAX(50, frame_rate) as f32);
    xx = 0.0;
    xy = 0.0;
    yy = 0.0;
    /* Unroll by 4. The frame size is always a multiple of 4 *except* for
       2.5 ms frames at 12 kHz. Since this setting is very rare (and very
       stupid), we just discard the last two samples. */
    let mut i = 0;
    while i < frame_size - 3 {
        let mut pxx: opus_val32 = 0.0;
        let mut pxy: opus_val32 = 0.0;
        let mut pyy: opus_val32 = 0.0;
        let mut x;
        let mut y;
        x = RES2VAL16(pcm[(2 * i) as usize]);
        y = RES2VAL16(pcm[(2 * i + 1) as usize]);
        pxx = SHR32(MULT16_16(x, x), 2);
        pxy = SHR32(MULT16_16(x, y), 2);
        pyy = SHR32(MULT16_16(y, y), 2);
        x = RES2VAL16(pcm[(2 * i + 2) as usize]);
        y = RES2VAL16(pcm[(2 * i + 3) as usize]);
        pxx += SHR32(MULT16_16(x, x), 2);
        pxy += SHR32(MULT16_16(x, y), 2);
        pyy += SHR32(MULT16_16(y, y), 2);
        x = RES2VAL16(pcm[(2 * i + 4) as usize]);
        y = RES2VAL16(pcm[(2 * i + 5) as usize]);
        pxx += SHR32(MULT16_16(x, x), 2);
        pxy += SHR32(MULT16_16(x, y), 2);
        pyy += SHR32(MULT16_16(y, y), 2);
        x = RES2VAL16(pcm[(2 * i + 6) as usize]);
        y = RES2VAL16(pcm[(2 * i + 7) as usize]);
        pxx += SHR32(MULT16_16(x, x), 2);
        pxy += SHR32(MULT16_16(x, y), 2);
        pyy += SHR32(MULT16_16(y, y), 2);

        xx += SHR32(pxx, 0);
        xy += SHR32(pxy, 0);
        yy += SHR32(pyy, 0);
        i += 4;
    }
    if !(xx < 1e9f32) || celt_isnan(xx) || !(yy < 1e9f32) || celt_isnan(yy) {
        xy = 0.0;
        xx = 0.0;
        yy = 0.0;
    }
    mem.XX += MULT16_32_Q15(short_alpha, xx - mem.XX);
    /*mem.XY += MULT16_32_Q15(short_alpha, xy-mem.XY);*/
    /* Rewritten to avoid overflows on abrupt sign change. */
    mem.XY = MULT16_32_Q15(Q15ONE - short_alpha, mem.XY) + MULT16_32_Q15(short_alpha, xy);
    mem.YY += MULT16_32_Q15(short_alpha, yy - mem.YY);
    mem.XX = MAX32(0.0, mem.XX);
    mem.XY = MAX32(0.0, mem.XY);
    mem.YY = MAX32(0.0, mem.YY);
    if MAX32(mem.XX, mem.YY) > QCONST16(8e-4f32, 18) {
        let corr;
        let ldiff;
        let width;
        sqrt_xx = celt_sqrt(mem.XX);
        sqrt_yy = celt_sqrt(mem.YY);
        qrrt_xx = celt_sqrt(sqrt_xx);
        qrrt_yy = celt_sqrt(sqrt_yy);
        /* Inter-channel correlation */
        mem.XY = MIN32(mem.XY, sqrt_xx * sqrt_yy);
        corr = SHR32(frac_div32(mem.XY, EPSILON + MULT16_16(sqrt_xx, sqrt_yy)), 16);
        /* Approximate loudness difference */
        ldiff = MULT16_16(Q15ONE, ABS16(qrrt_xx - qrrt_yy)) / (EPSILON + qrrt_xx + qrrt_yy);
        width = MULT16_16_Q15(
            MIN16(Q15ONE, celt_sqrt(QCONST32(1.0f32, 30) - MULT16_16(corr, corr))),
            ldiff,
        );
        /* Smoothing over one second */
        mem.smoothed_width += (width - mem.smoothed_width) / (frame_rate as f32);
        /* Peak follower */
        mem.max_follower = MAX16(
            mem.max_follower - QCONST16(0.02f32, 15) / (frame_rate as f32),
            mem.smoothed_width,
        );
    }
    EXTRACT16(MIN32(Q15ONE, MULT16_16(20.0, mem.max_follower)))
}

/* ------------------------------------------------------------------------- *
 * decide_fec / compute_silk_rate_for_hybrid / compute_equiv_rate
 * ------------------------------------------------------------------------- */

/// `decide_fec()` (`src/opus_encoder.c:960`).
fn decide_fec(
    useInBandFEC: i32,
    PacketLoss_perc: i32,
    last_fec: i32,
    mode: i32,
    bandwidth: &mut i32,
    rate: opus_int32,
) -> i32 {
    if useInBandFEC == 0 || PacketLoss_perc == 0 || mode == MODE_CELT_ONLY {
        return 0;
    }
    let orig_bandwidth = *bandwidth;
    loop {
        let mut LBRR_rate_thres_bps: opus_int32;
        /* Compute threshold for using FEC at the current bandwidth setting */
        LBRR_rate_thres_bps = fec_thresholds[(2 * (*bandwidth - OPUS_BANDWIDTH_NARROWBAND)) as usize];
        let hysteresis = fec_thresholds[(2 * (*bandwidth - OPUS_BANDWIDTH_NARROWBAND) + 1) as usize];
        if last_fec == 1 {
            LBRR_rate_thres_bps -= hysteresis;
        }
        if last_fec == 0 {
            LBRR_rate_thres_bps += hysteresis;
        }
        LBRR_rate_thres_bps = silk_SMULWB(
            silk_MUL(LBRR_rate_thres_bps, 125 - silk_min(PacketLoss_perc, 25)),
            SILK_FIX_CONST(0.01, 16),
        );
        /* If loss <= 5%, we look at whether we have enough rate to enable FEC.
           If loss > 5%, we decrease the bandwidth until we can enable FEC. */
        if rate > LBRR_rate_thres_bps {
            return 1;
        } else if PacketLoss_perc <= 5 {
            return 0;
        } else if *bandwidth > OPUS_BANDWIDTH_NARROWBAND {
            *bandwidth -= 1;
        } else {
            break;
        }
    }
    /* Couldn't find any bandwidth to enable FEC, keep original bandwidth. */
    *bandwidth = orig_bandwidth;
    0
}

/// `compute_silk_rate_for_hybrid()` (`src/opus_encoder.c:1000`).
fn compute_silk_rate_for_hybrid(
    mut rate: i32,
    bandwidth: i32,
    frame20ms: i32,
    vbr: i32,
    fec: i32,
    channels: i32,
) -> i32 {
    static rate_table: [[i32; 5]; 7] = [
        [0, 0, 0, 0, 0],
        [12000, 10000, 10000, 11000, 11000],
        [16000, 13500, 13500, 15000, 15000],
        [20000, 16000, 16000, 18000, 18000],
        [24000, 18000, 18000, 21000, 21000],
        [32000, 22000, 22000, 28000, 28000],
        [64000, 38000, 38000, 50000, 50000],
    ];
    /* Do the allocation per-channel. */
    rate /= channels;
    let entry = 1 + frame20ms + 2 * fec;
    let N = rate_table.len() as i32;
    let mut i = 1;
    while i < N {
        if rate_table[i as usize][0] > rate {
            break;
        }
        i += 1;
    }
    let silk_rate = if i == N {
        let mut silk_rate = rate_table[(i - 1) as usize][entry as usize];
        /* For now, just give 50% of the extra bits to SILK. */
        silk_rate += (rate - rate_table[(i - 1) as usize][0]) / 2;
        silk_rate
    } else {
        let lo = rate_table[(i - 1) as usize][entry as usize];
        let hi = rate_table[i as usize][entry as usize];
        let x0 = rate_table[(i - 1) as usize][0];
        let x1 = rate_table[i as usize][0];
        (lo * (x1 - rate) + hi * (rate - x0)) / (x1 - x0)
    };
    finish_silk_rate(silk_rate, bandwidth, vbr, channels, rate)
}

/// The tail of `compute_silk_rate_for_hybrid()` (split out to mirror the two
/// `return`s of the C without duplicating it).
#[inline(always)]
fn finish_silk_rate(mut silk_rate: i32, bandwidth: i32, vbr: i32, channels: i32, rate: i32) -> i32 {
    if vbr == 0 {
        /* Tiny boost to SILK for CBR. We should probably tune this better. */
        silk_rate += 100;
    }
    if bandwidth == OPUS_BANDWIDTH_SUPERWIDEBAND {
        silk_rate += 300;
    }
    silk_rate *= channels;
    /* Small adjustment for stereo (calibrated for 32 kb/s, haven't tried other bitrates). */
    if channels == 2 && rate >= 12000 {
        silk_rate -= 1000;
    }
    silk_rate
}

/// `compute_equiv_rate()` (`src/opus_encoder.c:1042`).
fn compute_equiv_rate(
    bitrate: opus_int32,
    channels: i32,
    frame_rate: i32,
    vbr: i32,
    mode: i32,
    complexity: i32,
    loss: i32,
) -> opus_int32 {
    let mut equiv = bitrate;
    /* Take into account overhead from smaller frames. */
    if frame_rate > 50 {
        equiv -= (40 * channels + 20) * (frame_rate - 50);
    }
    /* CBR is about a 8% penalty for both SILK and CELT. */
    if vbr == 0 {
        equiv -= equiv / 12;
    }
    /* Complexity makes about 10% difference (from 0 to 10) in general. */
    equiv = equiv * (90 + complexity) / 100;
    if mode == MODE_SILK_ONLY || mode == MODE_HYBRID {
        /* SILK complexity 0-1 uses the non-delayed-decision NSQ, which
           costs about 20%. */
        if complexity < 2 {
            equiv = equiv * 4 / 5;
        }
        equiv -= equiv * loss / (6 * loss + 10);
    } else if mode == MODE_CELT_ONLY {
        /* CELT complexity 0-4 doesn't have the pitch filter, which costs
           about 10%. */
        if complexity < 5 {
            equiv = equiv * 9 / 10;
        }
    } else {
        /* Mode not known yet */
        /* Half the SILK loss*/
        equiv -= equiv * loss / (12 * loss + 20);
    }
    equiv
}

/* ------------------------------------------------------------------------- *
 * is_digital_silence / compute_frame_energy / decide_dtx_mode
 * ------------------------------------------------------------------------- */

/// `is_digital_silence()` (`src/opus_encoder.c:1075`).
pub fn is_digital_silence(pcm: &[opus_res], frame_size: i32, channels: i32, lsb_depth: i32) -> i32 {
    let sample_max = celt_maxabs_res(pcm, frame_size * channels);
    (sample_max <= 1.0f32 / ((1i32 << lsb_depth) as f32)) as i32
}

/// `compute_frame_energy()` (`src/opus_encoder.c:1093`, `#else` float branch).
fn compute_frame_energy(pcm: &[opus_val16], frame_size: i32, channels: i32, arch: i32) -> opus_val32 {
    let _ = arch;
    let len = frame_size * channels;
    celt_inner_prod(pcm, pcm, len) / (len as f32)
}

/// `decide_dtx_mode()` (`src/opus_encoder.c:1120`).
fn decide_dtx_mode(activity: opus_int, nb_no_activity_ms_Q1: &mut i32, frame_size_ms_Q1: i32) -> i32 {
    if activity == 0 {
        /* The number of consecutive DTX frames should be within the allowed bounds.
           Note that the allowed bound is defined in the SILK headers and assumes 20 ms
           frames. As this function can be called with any frame length, a conversion to
           milliseconds is done before the comparisons. */
        *nb_no_activity_ms_Q1 += frame_size_ms_Q1;
        if *nb_no_activity_ms_Q1 > NB_SPEECH_FRAMES_BEFORE_DTX * 20 * 2 {
            if *nb_no_activity_ms_Q1 <= (NB_SPEECH_FRAMES_BEFORE_DTX + MAX_CONSECUTIVE_DTX) * 20 * 2
            {
                /* Valid frame for DTX! */
                return 1;
            } else {
                *nb_no_activity_ms_Q1 = NB_SPEECH_FRAMES_BEFORE_DTX * 20 * 2;
            }
        }
    } else {
        *nb_no_activity_ms_Q1 = 0;
    }
    0
}

/// `compute_redundancy_bytes()` (`src/opus_encoder.c:1145`).
fn compute_redundancy_bytes(
    max_data_bytes: opus_int32,
    bitrate_bps: opus_int32,
    frame_rate: i32,
    channels: i32,
) -> i32 {
    let base_bits = 40 * channels + 20;

    /* Equivalent rate for 5 ms frames. */
    let mut redundancy_rate = bitrate_bps + base_bits * (200 - frame_rate);
    /* For VBR, further increase the bitrate if we can afford it. It's pretty short
       and we'll avoid artefacts. */
    redundancy_rate = 3 * redundancy_rate / 2;
    let mut redundancy_bytes = redundancy_rate / 1600;

    /* Compute the max rate we can use given CBR or VBR with cap. */
    let available_bits = max_data_bytes * 8 - 2 * base_bits;
    let redundancy_bytes_cap = (available_bits * 240 / (240 + 48000 / frame_rate) + base_bits) / 8;
    redundancy_bytes = IMIN(redundancy_bytes, redundancy_bytes_cap);
    /* It we can't get enough bits for redundancy to be worth it, rely on the decoder PLC. */
    if redundancy_bytes > 4 + 8 * channels {
        redundancy_bytes = IMIN(257, redundancy_bytes);
    } else {
        redundancy_bytes = 0;
    }
    redundancy_bytes
}

/* ------------------------------------------------------------------------- *
 * Range-coder / data-buffer glue (see the module comment)
 * ------------------------------------------------------------------------- */

#[inline(always)]
fn enc_sync_in(enc: &mut ec_ctx, buf: &[u8]) {
    let n = enc.storage as usize;
    enc.buf[..n].copy_from_slice(&buf[1..1 + n]);
}

#[inline(always)]
fn enc_sync_out(enc: &ec_ctx, buf: &mut [u8]) {
    let n = enc.storage as usize;
    buf[1..1 + n].copy_from_slice(&enc.buf[..n]);
}

/* ------------------------------------------------------------------------- *
 * opus_encode_native
 * ------------------------------------------------------------------------- */

/// `opus_encode_native()` (`src/opus_encoder.c:1198`).
///
/// `analysis_pcm` carries the original (unconverted) PCM together with its
/// `downmix` function; C keeps them as a `const void*` and a function pointer.
#[allow(clippy::too_many_arguments)]
pub fn opus_encode_native<'a>(
    st: &mut OpusEncoder,
    pcm: &[opus_res],
    frame_size: i32,
    data: &mut [u8],
    out_data_bytes: opus_int32,
    mut lsb_depth: i32,
    analysis_pcm: Option<analysis::AnalysisPcm<'a>>,
    analysis_size: opus_int32,
    c1: i32,
    c2: i32,
    analysis_channels: i32,
    float_api: i32,
) -> i32 {
    let mut ret;
    let mut prefill = 0;
    let mut redundancy = 0;
    let mut celt_to_silk = 0;
    let mut to_celt = 0;
    let mut voice_est; /* Probability of voice in Q7 */
    let mut equiv_rate;
    let mut frame_rate;
    let max_rate; /* Max bitrate we're allowed to use */
    let mut curr_bandwidth;
    let mut max_data_bytes; /* Max number of bytes we're allowed to use */
    let mut cbr_bytes: opus_int32 = -1;
    let stereo_width;
    let packet_size_cap = 1276;
    let mut analysis_info = AnalysisInfo::default();
    let mut analysis_read_pos_bak = -1;
    let mut analysis_read_subframe_bak = -1;
    let mut is_silence;
    let dred_bitrate_bps;

    /* Just avoid insane packet sizes here, but the real bounds are applied later on. */
    max_data_bytes = IMIN(packet_size_cap * 6, out_data_bytes);

    st.rangeFinal = 0;
    if frame_size <= 0 || max_data_bytes <= 0 {
        return OPUS_BAD_ARG;
    }

    /* Cannot encode 100 ms in 1 byte */
    if max_data_bytes == 1 && st.Fs == (frame_size * 10) {
        return OPUS_BUFFER_TOO_SMALL;
    }

    lsb_depth = IMIN(lsb_depth, st.lsb_depth);

    is_silence = is_digital_silence(pcm, frame_size, st.channels, lsb_depth);
    analysis_info.valid = 0;
    if st.silk_mode.complexity >= 7
        && st.Fs >= 16000
        && st.Fs <= 48000
        && st.application != OPUS_APPLICATION_RESTRICTED_SILK
    {
        analysis_read_pos_bak = st.analysis.read_pos;
        analysis_read_subframe_bak = st.analysis.read_subframe;
        analysis::run_analysis(
            &mut st.analysis,
            &st.celt_enc.mode,
            analysis_pcm,
            analysis_size,
            frame_size,
            c1,
            c2,
            analysis_channels,
            st.Fs,
            lsb_depth,
            &mut analysis_info,
        );
    } else if st.analysis.initialized != 0 {
        st.analysis.tonality_analysis_reset();
    }

    /* Reset voice_ratio if this frame is not silent or if analysis is disabled.
     * Otherwise, preserve voice_ratio from the last non-silent frame */
    if is_silence == 0 {
        st.voice_ratio = -1;
    }
    st.detected_bandwidth = 0;
    if analysis_info.valid != 0 {
        let analysis_bandwidth;
        if st.signal_type == OPUS_AUTO {
            let prob;
            if st.prev_mode == 0 {
                prob = analysis_info.music_prob;
            } else if st.prev_mode == MODE_CELT_ONLY {
                prob = analysis_info.music_prob_max;
            } else {
                prob = analysis_info.music_prob_min;
            }
            st.voice_ratio = (0.5f64 + (100.0f32 * (1.0f32 - prob)) as f64).floor() as i32;
        }

        analysis_bandwidth = analysis_info.bandwidth;
        if analysis_bandwidth <= 12 {
            st.detected_bandwidth = OPUS_BANDWIDTH_NARROWBAND;
        } else if analysis_bandwidth <= 14 {
            st.detected_bandwidth = OPUS_BANDWIDTH_MEDIUMBAND;
        } else if analysis_bandwidth <= 16 {
            st.detected_bandwidth = OPUS_BANDWIDTH_WIDEBAND;
        } else if analysis_bandwidth <= 18 {
            st.detected_bandwidth = OPUS_BANDWIDTH_SUPERWIDEBAND;
        } else {
            st.detected_bandwidth = OPUS_BANDWIDTH_FULLBAND;
        }
    }

    /* Track the peak signal energy */
    if analysis_info.valid == 0 || analysis_info.activity_probability > DTX_ACTIVITY_THRESHOLD {
        if is_silence == 0 {
            st.peak_signal_energy = MAX32(
                MULT16_32_Q15(QCONST16(0.999f32, 15), st.peak_signal_energy),
                compute_frame_energy(pcm, frame_size, st.channels, st.arch),
            );
        }
    }
    if st.channels == 2 && st.force_channels != 1 {
        stereo_width = compute_stereo_width(pcm, frame_size, st.Fs, &mut st.width_mem);
    } else {
        stereo_width = 0.0;
    }
    st.bitrate_bps = user_bitrate_to_bitrate(st, frame_size, max_data_bytes);

    frame_rate = st.Fs / frame_size;
    if st.use_vbr == 0 {
        cbr_bytes = IMIN(
            (bitrate_to_bits(st.bitrate_bps, st.Fs, frame_size) + 4) / 8,
            max_data_bytes,
        );
        st.bitrate_bps = bits_to_bitrate(cbr_bytes * 8, st.Fs, frame_size);
        /* Make sure we provide at least one byte to avoid failing. */
        max_data_bytes = IMAX(1, cbr_bytes);
    }
    /* Allocate some of the bits to DRED if needed. */
    dred_bitrate_bps = compute_dred_bitrate(st, st.bitrate_bps, frame_size);
    st.bitrate_bps -= dred_bitrate_bps;
    if max_data_bytes < 3
        || st.bitrate_bps < 3 * frame_rate * 8
        || (frame_rate < 50 && (max_data_bytes * frame_rate < 300 || st.bitrate_bps < 2400))
    {
        /*If the space is too low to do something useful, emit 'PLC' frames.*/
        let mut tocmode = st.mode;
        let mut bw = if st.bandwidth == 0 {
            OPUS_BANDWIDTH_NARROWBAND
        } else {
            st.bandwidth
        };
        let mut packet_code = 0;
        let mut num_multiframes = 0;

        if tocmode == 0 {
            tocmode = MODE_SILK_ONLY;
        }
        if frame_rate > 100 {
            tocmode = MODE_CELT_ONLY;
        }
        /* 40 ms -> 2 x 20 ms if in CELT_ONLY or HYBRID mode */
        if frame_rate == 25 && tocmode != MODE_SILK_ONLY {
            frame_rate = 50;
            packet_code = 1;
        }

        /* >= 60 ms frames */
        if frame_rate <= 16 {
            /* 1 x 60 ms, 2 x 40 ms, 2 x 60 ms */
            if out_data_bytes == 1 || (tocmode == MODE_SILK_ONLY && frame_rate != 10) {
                tocmode = MODE_SILK_ONLY;

                packet_code = (frame_rate <= 12) as i32;
                frame_rate = if frame_rate == 12 { 25 } else { 16 };
            } else {
                num_multiframes = 50 / frame_rate;
                frame_rate = 50;
                packet_code = 3;
            }
        }

        if tocmode == MODE_SILK_ONLY && bw > OPUS_BANDWIDTH_WIDEBAND {
            bw = OPUS_BANDWIDTH_WIDEBAND;
        } else if tocmode == MODE_CELT_ONLY && bw == OPUS_BANDWIDTH_MEDIUMBAND {
            bw = OPUS_BANDWIDTH_NARROWBAND;
        } else if tocmode == MODE_HYBRID && bw <= OPUS_BANDWIDTH_SUPERWIDEBAND {
            bw = OPUS_BANDWIDTH_SUPERWIDEBAND;
        }

        data[0] = gen_toc(tocmode, frame_rate, bw, st.stream_channels);
        data[0] |= packet_code as u8;

        ret = if packet_code <= 1 { 1 } else { 2 };

        max_data_bytes = IMAX(max_data_bytes, ret);

        if packet_code == 3 {
            data[1] = num_multiframes as u8;
        }

        if st.use_vbr == 0 {
            ret = opus_packet_pad(data, ret, max_data_bytes);
            if ret == OPUS_OK {
                ret = max_data_bytes;
            } else {
                ret = OPUS_INTERNAL_ERROR;
            }
        }
        return ret;
    }
    max_rate = bits_to_bitrate(max_data_bytes * 8, st.Fs, frame_size);

    /* Equivalent 20-ms rate for mode/channel/bandwidth decisions */
    equiv_rate = compute_equiv_rate(
        st.bitrate_bps,
        st.channels,
        st.Fs / frame_size,
        st.use_vbr,
        0,
        st.silk_mode.complexity,
        st.silk_mode.packetLossPercentage,
    );

    if st.signal_type == OPUS_SIGNAL_VOICE {
        voice_est = 127;
    } else if st.signal_type == OPUS_SIGNAL_MUSIC {
        voice_est = 0;
    } else if st.voice_ratio >= 0 {
        voice_est = st.voice_ratio * 327 >> 8;
        /* For AUDIO, never be more than 90% confident of having speech */
        if st.application == OPUS_APPLICATION_AUDIO {
            voice_est = IMIN(voice_est, 115);
        }
    } else if st.application == OPUS_APPLICATION_VOIP {
        voice_est = 115;
    } else {
        voice_est = 48;
    }

    if st.force_channels != OPUS_AUTO && st.channels == 2 {
        st.stream_channels = st.force_channels;
    } else {
        /* Rate-dependent mono-stereo decision */
        if st.channels == 2 {
            let mut stereo_threshold = stereo_music_threshold
                + ((voice_est * voice_est * (stereo_voice_threshold - stereo_music_threshold)) >> 14);
            if st.stream_channels == 2 {
                stereo_threshold -= 1000;
            } else {
                stereo_threshold += 1000;
            }
            st.stream_channels = if equiv_rate > stereo_threshold { 2 } else { 1 };
        } else {
            st.stream_channels = st.channels;
        }
    }
    /* Update equivalent rate for channels decision. */
    equiv_rate = compute_equiv_rate(
        st.bitrate_bps,
        st.stream_channels,
        st.Fs / frame_size,
        st.use_vbr,
        0,
        st.silk_mode.complexity,
        st.silk_mode.packetLossPercentage,
    );

    /* Allow SILK DTX if DTX is enabled but the generalized DTX cannot be used,
       e.g. because of the complexity setting or sample rate. */
    st.silk_mode.useDTX = (st.use_dtx != 0 && !(analysis_info.valid != 0 || is_silence != 0)) as i32;

    /* Mode selection depending on application and signal type */
    if st.application == OPUS_APPLICATION_RESTRICTED_SILK {
        st.mode = MODE_SILK_ONLY;
    } else if st.application == OPUS_APPLICATION_RESTRICTED_LOWDELAY
        || st.application == OPUS_APPLICATION_RESTRICTED_CELT
    {
        st.mode = MODE_CELT_ONLY;
    } else if st.user_forced_mode == OPUS_AUTO {
        let mode_voice;
        let mode_music;
        let mut threshold;

        /* Interpolate based on stereo width */
        mode_voice = (MULT16_32_Q15(Q15ONE - stereo_width, mode_thresholds[0][0] as f32)
            + MULT16_32_Q15(stereo_width, mode_thresholds[1][0] as f32))
            as i32;
        mode_music = (MULT16_32_Q15(Q15ONE - stereo_width, mode_thresholds[1][1] as f32)
            + MULT16_32_Q15(stereo_width, mode_thresholds[1][1] as f32))
            as i32;
        /* Interpolate based on speech/music probability */
        threshold = mode_music + ((voice_est * voice_est * (mode_voice - mode_music)) >> 14);
        /* Bias towards SILK for VoIP because of some useful features */
        if st.application == OPUS_APPLICATION_VOIP {
            threshold += 8000;
        }

        /* Hysteresis */
        if st.prev_mode == MODE_CELT_ONLY {
            threshold -= 4000;
        } else if st.prev_mode > 0 {
            threshold += 4000;
        }

        st.mode = if equiv_rate >= threshold {
            MODE_CELT_ONLY
        } else {
            MODE_SILK_ONLY
        };

        /* When FEC is enabled and there's enough packet loss, use SILK.
           Unless the FEC is set to 2, in which case we don't switch to SILK if we're confident we have music. */
        if st.silk_mode.useInBandFEC != 0
            && st.silk_mode.packetLossPercentage > (128 - voice_est) >> 4
            && (st.fec_config != 2 || voice_est > 25)
        {
            st.mode = MODE_SILK_ONLY;
        }
        /* When encoding voice and DTX is enabled but the generalized DTX cannot be used,
           use SILK in order to make use of its DTX. */
        if st.silk_mode.useDTX != 0 && voice_est > 100 {
            st.mode = MODE_SILK_ONLY;
        }

        /* If max_data_bytes represents less than 6 kb/s, switch to CELT-only mode */
        if max_data_bytes
            < bitrate_to_bits(if frame_rate > 50 { 9000 } else { 6000 }, st.Fs, frame_size) / 8
        {
            st.mode = MODE_CELT_ONLY;
        }
    } else {
        st.mode = st.user_forced_mode;
    }

    /* Override the chosen mode to make sure we meet the requested frame size */
    if st.mode != MODE_CELT_ONLY && frame_size < st.Fs / 100 {
        celt_assert!(st.application != OPUS_APPLICATION_RESTRICTED_SILK);
        st.mode = MODE_CELT_ONLY;
    }
    if st.lfe != 0 && st.application != OPUS_APPLICATION_RESTRICTED_SILK {
        st.mode = MODE_CELT_ONLY;
    }

    if st.prev_mode > 0
        && ((st.mode != MODE_CELT_ONLY && st.prev_mode == MODE_CELT_ONLY)
            || (st.mode == MODE_CELT_ONLY && st.prev_mode != MODE_CELT_ONLY))
    {
        redundancy = 1;
        celt_to_silk = (st.mode != MODE_CELT_ONLY) as i32;
        if celt_to_silk == 0 {
            /* Switch to SILK/hybrid if frame size is 10 ms or more*/
            if frame_size >= st.Fs / 100 {
                st.mode = st.prev_mode;
                to_celt = 1;
            } else {
                redundancy = 0;
            }
        }
    }

    /* When encoding multiframes, we can ask for a switch to CELT only in the last frame. This switch
     * is processed above as the requested mode shouldn't interrupt stereo->mono transition. */
    if st.stream_channels == 1
        && st.prev_channels == 2
        && st.silk_mode.toMono == 0
        && st.mode != MODE_CELT_ONLY
        && st.prev_mode != MODE_CELT_ONLY
    {
        /* Delay stereo->mono transition by two frames so that SILK can do a smooth downmix */
        st.silk_mode.toMono = 1;
        st.stream_channels = 2;
    } else {
        st.silk_mode.toMono = 0;
    }

    /* Update equivalent rate with mode decision. */
    equiv_rate = compute_equiv_rate(
        st.bitrate_bps,
        st.stream_channels,
        st.Fs / frame_size,
        st.use_vbr,
        st.mode,
        st.silk_mode.complexity,
        st.silk_mode.packetLossPercentage,
    );

    if st.mode != MODE_CELT_ONLY && st.prev_mode == MODE_CELT_ONLY {
        let mut dummy = silk_EncControlStruct::default();
        silk_InitEncoder(&mut st.silk_enc, st.channels, st.arch, &mut dummy);
        prefill = 1;
    }

    /* Automatic (rate-dependent) bandwidth selection */
    if st.mode == MODE_CELT_ONLY || st.first != 0 || st.silk_mode.allowBandwidthSwitch != 0 {
        let voice_bandwidth_thresholds: &[opus_int32; 8];
        let music_bandwidth_thresholds: &[opus_int32; 8];
        let mut bandwidth_thresholds = [0 as opus_int32; 8];
        let mut bandwidth = OPUS_BANDWIDTH_FULLBAND;

        if st.channels == 2 && st.force_channels != 1 {
            voice_bandwidth_thresholds = &stereo_voice_bandwidth_thresholds;
            music_bandwidth_thresholds = &stereo_music_bandwidth_thresholds;
        } else {
            voice_bandwidth_thresholds = &mono_voice_bandwidth_thresholds;
            music_bandwidth_thresholds = &mono_music_bandwidth_thresholds;
        }
        /* Interpolate bandwidth thresholds depending on voice estimation */
        for i in 0..8 {
            bandwidth_thresholds[i] = music_bandwidth_thresholds[i]
                + ((voice_est
                    * voice_est
                    * (voice_bandwidth_thresholds[i] - music_bandwidth_thresholds[i]))
                    >> 14);
        }
        loop {
            let mut threshold;
            let hysteresis;
            threshold = bandwidth_thresholds[(2 * (bandwidth - OPUS_BANDWIDTH_MEDIUMBAND)) as usize];
            hysteresis =
                bandwidth_thresholds[(2 * (bandwidth - OPUS_BANDWIDTH_MEDIUMBAND) + 1) as usize];
            if st.first == 0 {
                if st.auto_bandwidth >= bandwidth {
                    threshold -= hysteresis;
                } else {
                    threshold += hysteresis;
                }
            }
            if equiv_rate >= threshold {
                break;
            }
            bandwidth -= 1;
            if bandwidth <= OPUS_BANDWIDTH_NARROWBAND {
                break;
            }
        }
        /* We don't use mediumband anymore, except when explicitly requested or during
           mode transitions. */
        if bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
            bandwidth = OPUS_BANDWIDTH_WIDEBAND;
        }
        st.bandwidth = bandwidth;
        st.auto_bandwidth = bandwidth;
        /* Prevents any transition to SWB/FB until the SILK layer has fully
           switched to WB mode and turned the variable LP filter off */
        if st.first == 0
            && st.mode != MODE_CELT_ONLY
            && st.silk_mode.inWBmodeWithoutVariableLP == 0
            && st.bandwidth > OPUS_BANDWIDTH_WIDEBAND
        {
            st.bandwidth = OPUS_BANDWIDTH_WIDEBAND;
        }
    }

    if st.bandwidth > st.max_bandwidth {
        st.bandwidth = st.max_bandwidth;
    }

    if st.user_bandwidth != OPUS_AUTO {
        st.bandwidth = st.user_bandwidth;
    }

    /* This prevents us from using hybrid at unsafe CBR/max rates */
    if st.mode != MODE_CELT_ONLY && max_rate < 15000 {
        st.bandwidth = IMIN(st.bandwidth, OPUS_BANDWIDTH_WIDEBAND);
    }

    /* Prevents Opus from wasting bits on frequencies that are above
       the Nyquist rate of the input signal */
    if st.Fs <= 24000 && st.bandwidth > OPUS_BANDWIDTH_SUPERWIDEBAND {
        st.bandwidth = OPUS_BANDWIDTH_SUPERWIDEBAND;
    }
    if st.Fs <= 16000 && st.bandwidth > OPUS_BANDWIDTH_WIDEBAND {
        st.bandwidth = OPUS_BANDWIDTH_WIDEBAND;
    }
    if st.Fs <= 12000 && st.bandwidth > OPUS_BANDWIDTH_MEDIUMBAND {
        st.bandwidth = OPUS_BANDWIDTH_MEDIUMBAND;
    }
    if st.Fs <= 8000 && st.bandwidth > OPUS_BANDWIDTH_NARROWBAND {
        st.bandwidth = OPUS_BANDWIDTH_NARROWBAND;
    }
    /* Use detected bandwidth to reduce the encoded bandwidth. */
    if st.detected_bandwidth != 0 && st.user_bandwidth == OPUS_AUTO {
        let min_detected_bandwidth;
        /* Makes bandwidth detection more conservative just in case the detector
           gets it wrong when we could have coded a high bandwidth transparently.
           When operating in SILK/hybrid mode, we don't go below wideband to avoid
           more complicated switches that require redundancy. */
        if equiv_rate <= 18000 * st.stream_channels && st.mode == MODE_CELT_ONLY {
            min_detected_bandwidth = OPUS_BANDWIDTH_NARROWBAND;
        } else if equiv_rate <= 24000 * st.stream_channels && st.mode == MODE_CELT_ONLY {
            min_detected_bandwidth = OPUS_BANDWIDTH_MEDIUMBAND;
        } else if equiv_rate <= 30000 * st.stream_channels {
            min_detected_bandwidth = OPUS_BANDWIDTH_WIDEBAND;
        } else if equiv_rate <= 44000 * st.stream_channels {
            min_detected_bandwidth = OPUS_BANDWIDTH_SUPERWIDEBAND;
        } else {
            min_detected_bandwidth = OPUS_BANDWIDTH_FULLBAND;
        }

        st.detected_bandwidth = IMAX(st.detected_bandwidth, min_detected_bandwidth);
        st.bandwidth = IMIN(st.bandwidth, st.detected_bandwidth);
    }
    st.silk_mode.LBRR_coded = decide_fec(
        st.silk_mode.useInBandFEC,
        st.silk_mode.packetLossPercentage,
        st.silk_mode.LBRR_coded,
        st.mode,
        &mut st.bandwidth,
        equiv_rate,
    );
    if st.application != OPUS_APPLICATION_RESTRICTED_SILK {
        let mut v = lsb_depth;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_LSB_DEPTH_REQUEST, &mut v);
    }

    /* CELT mode doesn't support mediumband, use wideband instead */
    if st.mode == MODE_CELT_ONLY && st.bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
        st.bandwidth = OPUS_BANDWIDTH_WIDEBAND;
    }
    if st.lfe != 0 {
        st.bandwidth = OPUS_BANDWIDTH_NARROWBAND;
    }

    curr_bandwidth = st.bandwidth;

    if st.application == OPUS_APPLICATION_RESTRICTED_SILK && curr_bandwidth > OPUS_BANDWIDTH_WIDEBAND
    {
        st.bandwidth = OPUS_BANDWIDTH_WIDEBAND;
        curr_bandwidth = OPUS_BANDWIDTH_WIDEBAND;
    }
    /* Chooses the appropriate mode for speech
       *NEVER* switch to/from CELT-only mode here as this will invalidate some assumptions */
    if st.mode == MODE_SILK_ONLY && curr_bandwidth > OPUS_BANDWIDTH_WIDEBAND {
        st.mode = MODE_HYBRID;
    }
    if st.mode == MODE_HYBRID && curr_bandwidth <= OPUS_BANDWIDTH_WIDEBAND {
        st.mode = MODE_SILK_ONLY;
    }

    /* Can't support higher than >60 ms frames, and >20 ms when in Hybrid or CELT-only modes */
    if (frame_size > st.Fs / 50 && (st.mode != MODE_SILK_ONLY)) || frame_size > 3 * st.Fs / 50 {
        let enc_frame_size;
        let nb_frames;
        let max_header_bytes;
        let repacketize_len;
        let max_len_sum;
        let mut tot_size: opus_int32 = 0;
        let mut tmp_len;
        let mut dtx_count = 0;
        let bak_to_mono;

        if st.mode == MODE_SILK_ONLY {
            if frame_size == 2 * st.Fs / 25 {
                /* 80 ms -> 2x 40 ms */
                enc_frame_size = st.Fs / 25;
            } else if frame_size == 3 * st.Fs / 25 {
                /* 120 ms -> 2x 60 ms */
                enc_frame_size = 3 * st.Fs / 50;
            } else {
                /* 100 ms -> 5x 20 ms */
                enc_frame_size = st.Fs / 50;
            }
        } else {
            enc_frame_size = st.Fs / 50;
        }

        nb_frames = frame_size / enc_frame_size;

        if analysis_read_pos_bak != -1 {
            /* Reset analysis position to the beginning of the first frame so we
               can use it one frame at a time. */
            st.analysis.read_pos = analysis_read_pos_bak;
            st.analysis.read_subframe = analysis_read_subframe_bak;
        }

        /* Worst cases:
         * 2 frames: Code 2 with different compressed sizes
         * >2 frames: Code 3 VBR */
        max_header_bytes = if nb_frames == 2 { 3 } else { 2 + (nb_frames - 1) * 2 };

        if st.use_vbr != 0 || st.user_bitrate_bps == OPUS_BITRATE_MAX {
            repacketize_len = out_data_bytes;
        } else {
            celt_assert!(cbr_bytes >= 0);
            repacketize_len = IMIN(cbr_bytes, out_data_bytes);
        }
        max_len_sum = nb_frames + repacketize_len - max_header_bytes;

        let mut tmp_data = vec![0u8; max_len_sum as usize];
        let mut rp = OpusRepacketizer::default();
        rp.init();

        bak_to_mono = st.silk_mode.toMono;
        if bak_to_mono != 0 {
            st.force_channels = 1;
        } else {
            st.prev_channels = st.stream_channels;
        }

        for i in 0..nb_frames {
            let first_frame;
            let frame_to_celt;
            let frame_redundancy;
            let mut curr_max;
            /* Attempt DRED encoding until we have a non-DTX frame. In case of DTX refresh,
               that allows for DRED not to be in the first frame. */
            first_frame = ((i == 0) || (i == dtx_count)) as i32;
            st.silk_mode.toMono = 0;
            st.nonfinal_frame = (i < (nb_frames - 1)) as i32;

            /* When switching from SILK/Hybrid to CELT, only ask for a switch at the last frame */
            frame_to_celt = (to_celt != 0 && i == nb_frames - 1) as i32;
            frame_redundancy =
                (redundancy != 0 && (frame_to_celt != 0 || (to_celt == 0 && i == 0))) as i32;

            curr_max = IMIN(
                bitrate_to_bits(st.bitrate_bps, st.Fs, enc_frame_size) / 8,
                max_len_sum / nb_frames,
            );
            curr_max = IMIN(
                curr_max,
                (max_len_sum - bitrate_to_bits(dred_bitrate_bps, st.Fs, frame_size) / 8)
                    / nb_frames,
            );
            if first_frame != 0 {
                curr_max += bitrate_to_bits(dred_bitrate_bps, st.Fs, frame_size) / 8;
            }
            curr_max = IMIN(max_len_sum - tot_size, curr_max);
            if analysis_read_pos_bak != -1 {
                /* Get analysis for current frame. */
                analysis::tonality_get_info(&mut st.analysis, &mut analysis_info, enc_frame_size);
            }
            is_silence = is_digital_silence(
                &pcm[(i * (st.channels * enc_frame_size)) as usize..],
                enc_frame_size,
                st.channels,
                lsb_depth,
            );

            let off = tot_size as usize;
            tmp_len = opus_encode_frame_native(
                st,
                &pcm[(i * (st.channels * enc_frame_size)) as usize..],
                enc_frame_size,
                &mut tmp_data[off..],
                curr_max,
                float_api,
                first_frame,
                dred_bitrate_bps,
                &analysis_info,
                is_silence,
                frame_redundancy,
                celt_to_silk,
                prefill,
                equiv_rate,
                frame_to_celt,
            );
            if tmp_len < 0 {
                return OPUS_INTERNAL_ERROR;
            } else if tmp_len == 1 {
                dtx_count += 1;
            }
            ret = rp.cat(&tmp_data[off..], tmp_len);

            if ret < 0 {
                return OPUS_INTERNAL_ERROR;
            }
            tot_size += tmp_len;
        }
        ret = rp.opus_repacketizer_out_range_impl(
            0,
            nb_frames,
            data,
            repacketize_len,
            0,
            (!(st.use_vbr != 0) && (dtx_count != nb_frames)) as i32,
            &[],
        );
        if ret < 0 {
            ret = OPUS_INTERNAL_ERROR;
        }
        st.silk_mode.toMono = bak_to_mono;
        return ret;
    } else {
        ret = opus_encode_frame_native(
            st,
            pcm,
            frame_size,
            data,
            max_data_bytes,
            float_api,
            1,
            dred_bitrate_bps,
            &analysis_info,
            is_silence,
            redundancy,
            celt_to_silk,
            prefill,
            equiv_rate,
            to_celt,
        );
        return ret;
    }
}

/* ------------------------------------------------------------------------- *
 * opus_encode_frame_native
 * ------------------------------------------------------------------------- */

/// `opus_encode_frame_native()` (`src/opus_encoder.c:1319`).
#[allow(clippy::too_many_arguments)]
fn opus_encode_frame_native(
    st: &mut OpusEncoder,
    pcm: &[opus_res],
    frame_size: i32,
    data: &mut [u8],
    orig_max_data_bytes: opus_int32,
    float_api: i32,
    first_frame: i32,
    dred_bitrate_bps: opus_int32,
    analysis_info: &AnalysisInfo,
    is_silence: i32,
    mut redundancy: i32,
    mut celt_to_silk: i32,
    mut prefill: i32,
    equiv_rate: opus_int32,
    to_celt: i32,
) -> i32 {
    let mut ret = 0;
    let max_data_bytes;
    let mut nBytes: opus_int32 = 0;
    let mut redundancy_bytes = 0; /* Number of bytes to use for redundancy frame */
    let mut nb_compr_bytes;
    let mut redundant_rng: opus_uint32 = 0;
    let cutoff_Hz;
    let hp_freq_smth1;
    let mut HB_gain;
    let mut apply_padding;
    let frame_rate;
    let mut curr_bandwidth;
    let delay_compensation;
    let total_buffer;
    let mut activity: opus_int = VAD_NO_DECISION;

    max_data_bytes = IMIN(orig_max_data_bytes, 1276);
    st.rangeFinal = 0;
    let have_celt = st.application != OPUS_APPLICATION_RESTRICTED_SILK;
    curr_bandwidth = st.bandwidth;
    if st.application == OPUS_APPLICATION_RESTRICTED_LOWDELAY
        || st.application == OPUS_APPLICATION_RESTRICTED_CELT
        || st.application == OPUS_APPLICATION_RESTRICTED_SILK
    {
        delay_compensation = 0;
    } else {
        delay_compensation = st.delay_compensation;
    }
    total_buffer = delay_compensation;

    frame_rate = st.Fs / frame_size;

    if is_silence != 0 {
        activity = (!(is_silence != 0)) as i32;
    } else if analysis_info.valid != 0 {
        activity = (analysis_info.activity_probability >= DTX_ACTIVITY_THRESHOLD) as i32;
        if activity == 0 {
            /* Mark as active if this noise frame is sufficiently loud */
            let noise_energy = compute_frame_energy(pcm, frame_size, st.channels, st.arch);
            activity = (st.peak_signal_energy < (PSEUDO_SNR_THRESHOLD * noise_energy)) as i32;
        }
    } else if st.mode == MODE_CELT_ONLY {
        let noise_energy = compute_frame_energy(pcm, frame_size, st.channels, st.arch);
        /* Boosting peak energy a bit because we didn't just average the active frames. */
        activity =
            (st.peak_signal_energy < (QCONST16(PSEUDO_SNR_THRESHOLD, 0) * HALF32(noise_energy)))
                as i32;
    }

    /* For the first frame at a new SILK bandwidth */
    if st.silk_bw_switch != 0 {
        redundancy = 1;
        celt_to_silk = 1;
        st.silk_bw_switch = 0;
        /* Do a prefill without resetting the sampling rate control. */
        prefill = 2;
    }

    /* If we decided to go with CELT, make sure redundancy is off, no matter what
       we decided earlier. */
    if st.mode == MODE_CELT_ONLY {
        redundancy = 0;
    }

    if redundancy != 0 {
        redundancy_bytes =
            compute_redundancy_bytes(max_data_bytes, st.bitrate_bps, frame_rate, st.stream_channels);
        if redundancy_bytes == 0 {
            redundancy = 0;
        }
    }
    if st.application == OPUS_APPLICATION_RESTRICTED_SILK {
        redundancy = 0;
        redundancy_bytes = 0;
    }

    let bits_target = IMIN(
        8 * (max_data_bytes - redundancy_bytes),
        bitrate_to_bits(st.bitrate_bps, st.Fs, frame_size),
    ) - 8;

    /* C: `data += 1`; `data[k]` is `buf[1+k]` from here until `data--` below.
       `enc` mirrors the range coder that C writes into `data[1..]`. */
    let mut enc = ec_enc_init((orig_max_data_bytes - 1) as u32);

    let mut pcm_buf = vec![0.0f32; ((total_buffer + frame_size) * st.channels) as usize];
    {
        let n = (total_buffer * st.channels) as usize;
        let start = ((st.encoder_buffer - total_buffer) * st.channels) as usize;
        pcm_buf[..n].copy_from_slice(&st.delay_buffer[start..start + n]);
    }

    if st.mode == MODE_CELT_ONLY {
        hp_freq_smth1 = silk_LSHIFT(silk_lin2log(VARIABLE_HP_MIN_CUTOFF_HZ), 8);
    } else {
        hp_freq_smth1 = st.silk_enc.state_Fxx[0].sCmn.variable_HP_smth1_Q15;
    }

    st.variable_HP_smth2_Q15 = silk_SMLAWB(
        st.variable_HP_smth2_Q15,
        hp_freq_smth1 - st.variable_HP_smth2_Q15,
        SILK_FIX_CONST(VARIABLE_HP_SMTH_COEF2, 16),
    );

    /* convert from log scale to Hertz */
    cutoff_Hz = silk_log2lin(silk_RSHIFT(st.variable_HP_smth2_Q15, 8));

    let pcm_off = (total_buffer * st.channels) as usize;
    if st.application == OPUS_APPLICATION_VOIP {
        hp_cutoff(
            pcm,
            cutoff_Hz,
            &mut pcm_buf[pcm_off..],
            &mut st.hp_mem,
            frame_size,
            st.channels,
            st.Fs,
            st.arch,
        );
    } else {
        dc_reject(
            pcm,
            3,
            &mut pcm_buf[pcm_off..],
            &mut st.hp_mem,
            frame_size,
            st.channels,
            st.Fs,
        );
    }
    if float_api != 0 {
        let sum = celt_inner_prod(&pcm_buf[pcm_off..], &pcm_buf[pcm_off..], frame_size * st.channels);
        /* This should filter out both NaNs and ridiculous signals that could
           cause NaNs further down. */
        if !(sum < 1e9f32) || celt_isnan(sum) {
            let n = (frame_size * st.channels) as usize;
            for v in pcm_buf[pcm_off..pcm_off + n].iter_mut() {
                *v = 0.0;
            }
            st.hp_mem[0] = 0.0;
            st.hp_mem[1] = 0.0;
            st.hp_mem[2] = 0.0;
            st.hp_mem[3] = 0.0;
        }
    }

    /* Compute the DRED features. Needs to be before SILK because of DTX. */
    if st.dred_duration > 0 && st.dred_encoder.loaded != 0 {
        let frame_size_400Hz = frame_size * 400 / st.Fs;
        dred_compute_latents(
            &mut st.dred_encoder,
            &pcm_buf[pcm_off..],
            frame_size,
            total_buffer,
            st.arch,
        );
        let n = (4 * DRED_MAX_FRAMES - frame_size_400Hz) as usize;
        st.activity_mem.copy_within(0..n, frame_size_400Hz as usize);
        for i in 0..frame_size_400Hz as usize {
            st.activity_mem[i] = activity as u8;
        }
    } else {
        st.dred_encoder.latents_buffer_fill = 0;
        for v in st.activity_mem.iter_mut() {
            *v = 0;
        }
    }

    /* SILK processing */
    HB_gain = Q15ONE;
    if st.mode != MODE_CELT_ONLY {
        let total_bitRate;
        let celt_rate;
        let mut pcm_silk;

        /* Distribute bits between SILK and CELT */
        total_bitRate = bits_to_bitrate(bits_target, st.Fs, frame_size);
        if st.mode == MODE_HYBRID {
            /* Base rate for SILK */
            st.silk_mode.bitRate = compute_silk_rate_for_hybrid(
                total_bitRate,
                curr_bandwidth,
                (st.Fs == 50 * frame_size) as i32,
                st.use_vbr,
                st.silk_mode.LBRR_coded,
                st.stream_channels,
            );
            if st.energy_masking.is_none() {
                /* Increasingly attenuate high band when it gets allocated fewer bits */
                celt_rate = total_bitRate - st.silk_mode.bitRate;
                HB_gain = Q15ONE
                    - SHR32(celt_exp2((-(celt_rate as f32)) * QCONST16(1.0f32 / 1024.0, 10)), 1);
            }
        } else {
            /* SILK gets all bits */
            st.silk_mode.bitRate = total_bitRate;
        }

        /* Surround masking for SILK */
        if st.energy_masking.is_some() && st.use_vbr != 0 && st.lfe == 0 {
            let mut mask_sum: opus_val32 = 0.0;
            let mut end = 17;
            let mut srate: opus_int16 = 16000;
            if st.bandwidth == OPUS_BANDWIDTH_NARROWBAND {
                end = 13;
                srate = 8000;
            } else if st.bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
                end = 15;
                srate = 12000;
            }
            let mask = st.energy_masking.as_ref().unwrap();
            for c in 0..st.channels {
                for i in 0..end {
                    let mut m: celt_glog = MAXG(
                        MING(mask[(21 * c + i) as usize], GCONST(0.5f32)),
                        -GCONST(2.0f32),
                    );
                    if m > 0.0 {
                        m = HALF32(m);
                    }
                    mask_sum += m;
                }
            }
            /* Conservative rate reduction, we cut the masking in half */
            let mut masking_depth: celt_glog = mask_sum / (end as f32) * (st.channels as f32);
            masking_depth += GCONST(0.2f32);
            let mut rate_offset: opus_int32 =
                PSHR32(MULT16_16(srate as f32, SHR32(masking_depth, DB_SHIFT - 10)), 10) as i32;
            rate_offset = MAX32(rate_offset, -2 * st.silk_mode.bitRate / 3);
            /* Split the rate change between the SILK and CELT part for hybrid. */
            if st.bandwidth == OPUS_BANDWIDTH_SUPERWIDEBAND || st.bandwidth == OPUS_BANDWIDTH_FULLBAND {
                st.silk_mode.bitRate += 3 * rate_offset / 5;
            } else {
                st.silk_mode.bitRate += rate_offset;
            }
        }

        st.silk_mode.payloadSize_ms = 1000 * frame_size / st.Fs;
        st.silk_mode.nChannelsAPI = st.channels;
        st.silk_mode.nChannelsInternal = st.stream_channels;
        if curr_bandwidth == OPUS_BANDWIDTH_NARROWBAND {
            st.silk_mode.desiredInternalSampleRate = 8000;
        } else if curr_bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
            st.silk_mode.desiredInternalSampleRate = 12000;
        } else {
            celt_assert!(st.mode == MODE_HYBRID || curr_bandwidth == OPUS_BANDWIDTH_WIDEBAND);
            st.silk_mode.desiredInternalSampleRate = 16000;
        }
        if st.mode == MODE_HYBRID {
            /* Don't allow bandwidth reduction at lowest bitrates in hybrid mode */
            st.silk_mode.minInternalSampleRate = 16000;
        } else {
            st.silk_mode.minInternalSampleRate = 8000;
        }

        st.silk_mode.maxInternalSampleRate = 16000;
        if st.mode == MODE_SILK_ONLY {
            let mut effective_max_rate = bits_to_bitrate(max_data_bytes * 8, st.Fs, frame_size);
            if frame_rate > 50 {
                effective_max_rate = effective_max_rate * 2 / 3;
            }
            if effective_max_rate < 8000 {
                st.silk_mode.maxInternalSampleRate = 12000;
                st.silk_mode.desiredInternalSampleRate =
                    IMIN(12000, st.silk_mode.desiredInternalSampleRate);
            }
            if effective_max_rate < 7000 {
                st.silk_mode.maxInternalSampleRate = 8000;
                st.silk_mode.desiredInternalSampleRate =
                    IMIN(8000, st.silk_mode.desiredInternalSampleRate);
            }
        }

        st.silk_mode.useCBR = (!(st.use_vbr != 0)) as i32;

        /* Call SILK encoder for the low band */

        /* Max bits for SILK, counting ToC, redundancy bytes, and optionally redundancy. */
        st.silk_mode.maxBits = (max_data_bytes - 1) * 8;
        if redundancy != 0 && redundancy_bytes >= 2 {
            /* Counting 1 bit for redundancy position and 20 bits for flag+size (only for hybrid). */
            st.silk_mode.maxBits -= redundancy_bytes * 8 + 1;
            if st.mode == MODE_HYBRID {
                st.silk_mode.maxBits -= 20;
            }
        }
        if st.silk_mode.useCBR != 0 {
            /* When we're in CBR mode, but we have non-SILK data to encode, switch SILK to VBR with cap to
               save on complexity. Any variations will be absorbed by CELT and/or DRED and we can still
               produce a constant bitrate without wasting bits. */
            if st.mode == MODE_HYBRID || dred_bitrate_bps > 0 {
                /* Allow SILK to steal up to 25% of the remaining bits */
                let other_bits: opus_int16 =
                    IMAX(0, st.silk_mode.maxBits - st.silk_mode.bitRate * frame_size / st.Fs) as i16;
                st.silk_mode.maxBits = IMAX(0, st.silk_mode.maxBits - (other_bits as i32) * 3 / 4);
                st.silk_mode.useCBR = 0;
            }
        } else {
            /* Constrained VBR. */
            if st.mode == MODE_HYBRID {
                /* Compute SILK bitrate corresponding to the max total bits available */
                let maxBitRate = compute_silk_rate_for_hybrid(
                    st.silk_mode.maxBits * st.Fs / frame_size,
                    curr_bandwidth,
                    (st.Fs == 50 * frame_size) as i32,
                    st.use_vbr,
                    st.silk_mode.LBRR_coded,
                    st.stream_channels,
                );
                st.silk_mode.maxBits = bitrate_to_bits(maxBitRate, st.Fs, frame_size);
            }
        }

        if prefill != 0 && st.application != OPUS_APPLICATION_RESTRICTED_SILK {
            let mut zero: opus_int32 = 0;
            /* Use a smooth onset for the SILK prefill to avoid the encoder trying to encode
               a discontinuity. The exact location is what we need to avoid leaving any "gap"
               in the audio when mixing with the redundant CELT frame. Here we can afford to
               overwrite st->delay_buffer because the only thing that uses it before it gets
               rewritten is tmp_prefill[] and even then only the part after the ramp really
               gets used (rather than sent to the encoder and discarded) */
            let prefill_offset = (st.channels
                * (st.encoder_buffer - st.delay_compensation - st.Fs / 400)) as usize;
            let overlap = st.celt_enc.mode.overlap;
            let window = st.celt_enc.mode.window.clone();
            gain_fade(
                &mut st.delay_buffer[prefill_offset..],
                0.0,
                Q15ONE,
                overlap,
                st.Fs / 400,
                st.channels,
                &window,
                st.Fs,
            );
            for v in st.delay_buffer[..prefill_offset].iter_mut() {
                *v = 0.0;
            }
            pcm_silk = &st.delay_buffer[..];
            let mut dummy = ec_enc_init(0);
            silk_Encode(
                &mut st.silk_enc,
                &mut st.silk_mode,
                pcm_silk,
                st.encoder_buffer,
                &mut dummy,
                &mut zero,
                prefill,
                activity,
            );
            /* Prevent a second switch in the real encode call. */
            st.silk_mode.opusCanSwitch = 0;
        }

        pcm_silk = &pcm_buf[pcm_off..];
        enc_sync_in(&mut enc, data);
        ret = silk_Encode(
            &mut st.silk_enc,
            &mut st.silk_mode,
            pcm_silk,
            frame_size,
            &mut enc,
            &mut nBytes,
            0,
            activity,
        );
        enc_sync_out(&enc, data);
        if ret != 0 {
            /*fprintf (stderr, "SILK encode error: %d\n", ret);*/
            /* Handle error */
            return OPUS_INTERNAL_ERROR;
        }

        /* Extract SILK internal bandwidth for signaling in first byte */
        if st.mode == MODE_SILK_ONLY {
            if st.silk_mode.internalSampleRate == 8000 {
                curr_bandwidth = OPUS_BANDWIDTH_NARROWBAND;
            } else if st.silk_mode.internalSampleRate == 12000 {
                curr_bandwidth = OPUS_BANDWIDTH_MEDIUMBAND;
            } else if st.silk_mode.internalSampleRate == 16000 {
                curr_bandwidth = OPUS_BANDWIDTH_WIDEBAND;
            }
        } else {
            celt_assert!(st.silk_mode.internalSampleRate == 16000);
        }

        st.silk_mode.opusCanSwitch = (st.silk_mode.switchReady != 0 && st.nonfinal_frame == 0) as i32;

        if activity == VAD_NO_DECISION {
            activity = (st.silk_mode.signalType != TYPE_NO_VOICE_ACTIVITY) as i32;
            for i in 0..(frame_size * 400 / st.Fs) as usize {
                st.activity_mem[i] = activity as u8;
            }
        }
        if nBytes == 0 {
            st.rangeFinal = 0;
            data[0] = gen_toc(st.mode, st.Fs / frame_size, curr_bandwidth, st.stream_channels);
            return 1;
        }

        /* FIXME: How do we allocate the redundancy for CBR? */
        if st.silk_mode.opusCanSwitch != 0 {
            if st.application != OPUS_APPLICATION_RESTRICTED_SILK {
                redundancy_bytes = compute_redundancy_bytes(
                    max_data_bytes,
                    st.bitrate_bps,
                    frame_rate,
                    st.stream_channels,
                );
                redundancy = (redundancy_bytes != 0) as i32;
            }
            celt_to_silk = 0;
            st.silk_bw_switch = 1;
        }
    }

    /* CELT processing */
    if have_celt {
        let mut endband = 21;

        match curr_bandwidth {
            OPUS_BANDWIDTH_NARROWBAND => endband = 13,
            OPUS_BANDWIDTH_MEDIUMBAND | OPUS_BANDWIDTH_WIDEBAND => endband = 17,
            OPUS_BANDWIDTH_SUPERWIDEBAND => endband = 19,
            OPUS_BANDWIDTH_FULLBAND => endband = 21,
            _ => {}
        }
        let mut v = endband;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_END_BAND_REQUEST, &mut v);
        let mut v = st.stream_channels;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_CHANNELS_REQUEST, &mut v);
        let mut v = OPUS_BITRATE_MAX;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_BITRATE_REQUEST, &mut v);
    }
    if st.mode != MODE_SILK_ONLY {
        let mut celt_pred = 2.0f32;
        /* We may still decide to disable prediction later */
        if st.silk_mode.reducedDependency != 0 {
            celt_pred = 0.0;
        }
        let mut v = celt_pred as i32;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_PREDICTION_REQUEST, &mut v);
    }

    let mut tmp_prefill = vec![0.0f32; (st.channels * st.Fs / 400) as usize];
    if st.mode != MODE_SILK_ONLY
        && st.mode != st.prev_mode
        && st.prev_mode > 0
        && st.application != OPUS_APPLICATION_RESTRICTED_CELT
    {
        let n = (st.channels * st.Fs / 400) as usize;
        let start = ((st.encoder_buffer - total_buffer - st.Fs / 400) * st.channels) as usize;
        tmp_prefill[..n].copy_from_slice(&st.delay_buffer[start..start + n]);
    }

    if st.channels * (st.encoder_buffer - (frame_size + total_buffer)) > 0 {
        let n = (st.channels * (st.encoder_buffer - frame_size - total_buffer)) as usize;
        let src = (st.channels * frame_size) as usize;
        st.delay_buffer.copy_within(src..src + n, 0);
        let n2 = ((frame_size + total_buffer) * st.channels) as usize;
        let dst = (st.channels * (st.encoder_buffer - frame_size - total_buffer)) as usize;
        st.delay_buffer[dst..dst + n2].copy_from_slice(&pcm_buf[..n2]);
    } else {
        let n = (st.encoder_buffer * st.channels) as usize;
        let src = ((frame_size + total_buffer - st.encoder_buffer) * st.channels) as usize;
        st.delay_buffer[..n].copy_from_slice(&pcm_buf[src..src + n]);
    }
    /* gain_fade() and stereo_fade() need to be after the buffer copying
       because we don't want any of this to affect the SILK part */
    if (st.prev_HB_gain < Q15ONE || HB_gain < Q15ONE) && have_celt {
        let overlap = st.celt_enc.mode.overlap;
        let window = st.celt_enc.mode.window.clone();
        gain_fade(
            &mut pcm_buf,
            st.prev_HB_gain,
            HB_gain,
            overlap,
            frame_size,
            st.channels,
            &window,
            st.Fs,
        );
    }
    st.prev_HB_gain = HB_gain;
    if st.mode != MODE_HYBRID || st.stream_channels == 1 {
        if equiv_rate > 32000 {
            st.silk_mode.stereoWidth_Q14 = 16384;
        } else if equiv_rate < 16000 {
            st.silk_mode.stereoWidth_Q14 = 0;
        } else {
            st.silk_mode.stereoWidth_Q14 =
                16384 - 2048 * (32000 - equiv_rate) / (equiv_rate - 14000);
        }
    }
    if st.energy_masking.is_none() && st.channels == 2 {
        /* Apply stereo width reduction (at low bitrates) */
        if st.hybrid_stereo_width_Q14 < (1i16 << 14) || st.silk_mode.stereoWidth_Q14 < (1 << 14) {
            let mut g1;
            let mut g2;
            g1 = st.hybrid_stereo_width_Q14 as f32;
            g2 = st.silk_mode.stereoWidth_Q14 as f32;
            g1 *= 1.0f32 / 16384.0;
            g2 *= 1.0f32 / 16384.0;
            if have_celt {
                let overlap = st.celt_enc.mode.overlap;
                let window = st.celt_enc.mode.window.clone();
                stereo_fade(
                    &mut pcm_buf,
                    g1,
                    g2,
                    overlap,
                    frame_size,
                    st.channels,
                    &window,
                    st.Fs,
                );
            }
            st.hybrid_stereo_width_Q14 = st.silk_mode.stereoWidth_Q14 as i16;
        }
    }

    if st.mode != MODE_CELT_ONLY
        && enc.ec_tell() + 17 + 20 * ((st.mode == MODE_HYBRID) as i32) <= 8 * (max_data_bytes - 1)
    {
        /* For SILK mode, the redundancy is inferred from the length */
        if st.mode == MODE_HYBRID {
            enc_sync_in(&mut enc, data);
            ec_enc_bit_logp(&mut enc, redundancy, 12);
            enc_sync_out(&enc, data);
        }
        if redundancy != 0 {
            let max_redundancy;
            enc_sync_in(&mut enc, data);
            ec_enc_bit_logp(&mut enc, celt_to_silk, 1);
            enc_sync_out(&enc, data);
            if st.mode == MODE_HYBRID {
                /* Reserve the 8 bits needed for the redundancy length,
                   and at least a few bits for CELT if possible */
                max_redundancy = (max_data_bytes - 1) - ((enc.ec_tell() + 8 + 3 + 7) >> 3);
            } else {
                max_redundancy = (max_data_bytes - 1) - ((enc.ec_tell() + 7) >> 3);
            }
            /* Target the same bit-rate for redundancy as for the rest,
               up to a max of 257 bytes */
            redundancy_bytes = IMIN(max_redundancy, redundancy_bytes);
            redundancy_bytes = IMIN(257, IMAX(2, redundancy_bytes));
            if st.mode == MODE_HYBRID {
                enc_sync_in(&mut enc, data);
                ec_enc_uint(&mut enc, (redundancy_bytes - 2) as u32, 256);
                enc_sync_out(&enc, data);
            }
        }
    } else {
        redundancy = 0;
    }

    if redundancy == 0 {
        st.silk_bw_switch = 0;
        redundancy_bytes = 0;
    }
    let mut start_band = 0;
    if st.mode != MODE_CELT_ONLY {
        start_band = 17;
    }

    if st.mode == MODE_SILK_ONLY {
        ret = (enc.ec_tell() + 7) >> 3;
        enc_sync_in(&mut enc, data);
        ec_enc_done(&mut enc);
        enc_sync_out(&enc, data);
        nb_compr_bytes = ret;
    } else {
        nb_compr_bytes = (max_data_bytes - 1) - redundancy_bytes;
        if st.dred_duration > 0 {
            let dred_bytes = bitrate_to_bits(dred_bitrate_bps, st.Fs, frame_size) / 8;
            /* Allow CELT to steal up to 25% of the remaining bits. */
            let mut max_celt_bytes = nb_compr_bytes - dred_bytes * 3 / 4;
            /* But try to give CELT at least 5 bytes to prevent a mismatch with
               the redundancy signaling. */
            max_celt_bytes = IMAX((enc.ec_tell() + 7) / 8 + 5, max_celt_bytes);
            /* Subject to the original max. */
            nb_compr_bytes = IMIN(nb_compr_bytes, max_celt_bytes);
        }
        enc_sync_in(&mut enc, data);
        ec_enc_shrink(&mut enc, nb_compr_bytes as u32);
        enc_sync_out(&enc, data);
    }

    if redundancy != 0 || st.mode != MODE_SILK_ONLY {
        opus_custom_encoder_ctl_analysis(&mut st.celt_enc, Some(analysis_info));
    }
    if st.mode == MODE_HYBRID {
        let info = crate::celt::celt::SILKInfo {
            signalType: st.silk_mode.signalType,
            offset: st.silk_mode.offset,
        };
        opus_custom_encoder_ctl_silk_info(&mut st.celt_enc, Some(&info));
    }

    /* 5 ms redundant frame for CELT->SILK */
    if redundancy != 0 && celt_to_silk != 0 {
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_START_BAND_REQUEST, &mut v);
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_REQUEST, &mut v);
        let mut v = OPUS_BITRATE_MAX;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_BITRATE_REQUEST, &mut v);
        let err = celt_encode_with_ec(
            &mut st.celt_enc,
            &pcm_buf,
            st.Fs / 200,
            &mut data[1 + nb_compr_bytes as usize..],
            redundancy_bytes,
            None,
        );
        if err < 0 {
            return OPUS_INTERNAL_ERROR;
        }
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        redundant_rng = v as opus_uint32;
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_RESET_STATE, &mut v);
    }

    if have_celt {
        let mut v = start_band;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_START_BAND_REQUEST, &mut v);
    }

    data[0] = 0;
    if st.mode != MODE_SILK_ONLY {
        let mut v = st.use_vbr;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_REQUEST, &mut v);
        if st.mode == MODE_HYBRID {
            if st.use_vbr != 0 {
                let mut v = st.bitrate_bps - st.silk_mode.bitRate;
                celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_BITRATE_REQUEST, &mut v);
                let mut v = 0;
                celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut v);
            }
        } else {
            if st.use_vbr != 0 {
                let mut v = 1;
                celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_REQUEST, &mut v);
                let mut v = st.vbr_constraint;
                celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut v);
                let mut v = st.bitrate_bps;
                celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_BITRATE_REQUEST, &mut v);
            }
        }
        /* When Using DRED CBR, we can actually make the CELT part VBR and have DRED pick up the slack. */
        if st.use_vbr == 0 && st.dred_duration > 0 {
            let mut celt_bitrate = st.bitrate_bps;
            let mut v = 1;
            celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_REQUEST, &mut v);
            let mut v = 0;
            celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_CONSTRAINT_REQUEST, &mut v);
            if st.mode == MODE_HYBRID {
                celt_bitrate -= st.silk_mode.bitRate;
            }
            let mut v = celt_bitrate;
            celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_BITRATE_REQUEST, &mut v);
        }
        if st.mode != st.prev_mode
            && st.prev_mode > 0
            && st.application != OPUS_APPLICATION_RESTRICTED_CELT
        {
            let mut dummy = [0u8; 2];
            let mut v = 0;
            celt_encoder_ctl(&mut st.celt_enc, OPUS_RESET_STATE, &mut v);

            /* Prefilling */
            celt_encode_with_ec(&mut st.celt_enc, &tmp_prefill, st.Fs / 400, &mut dummy, 2, None);
            let mut v = 0;
            celt_encoder_ctl(&mut st.celt_enc, CELT_SET_PREDICTION_REQUEST, &mut v);
        }
        /* If false, we already busted the budget and we'll end up with a "PLC frame" */
        if enc.ec_tell() <= 8 * nb_compr_bytes {
            enc_sync_in(&mut enc, data);
            ret = celt_encode_with_ec(
                &mut st.celt_enc,
                &pcm_buf,
                frame_size,
                &mut [],
                nb_compr_bytes,
                Some(&mut enc),
            );
            enc_sync_out(&enc, data);
            if ret < 0 {
                return OPUS_INTERNAL_ERROR;
            }
            /* Put CELT->SILK redundancy data in the right place. */
            if redundancy != 0 && celt_to_silk != 0 && st.mode == MODE_HYBRID && nb_compr_bytes != ret
            {
                let n = redundancy_bytes as usize;
                data.copy_within(
                    1 + nb_compr_bytes as usize..1 + nb_compr_bytes as usize + n,
                    1 + ret as usize,
                );
                nb_compr_bytes = ret + redundancy_bytes;
            }
        }
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        st.rangeFinal = v as opus_uint32;
    } else {
        st.rangeFinal = enc.rng;
    }

    /* 5 ms redundant frame for SILK->CELT */
    if redundancy != 0 && celt_to_silk == 0 {
        let mut dummy = [0u8; 2];
        let N2 = st.Fs / 200;
        let N4 = st.Fs / 400;

        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_RESET_STATE, &mut v);
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_START_BAND_REQUEST, &mut v);
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, CELT_SET_PREDICTION_REQUEST, &mut v);
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_VBR_REQUEST, &mut v);
        let mut v = OPUS_BITRATE_MAX;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_SET_BITRATE_REQUEST, &mut v);

        if st.mode == MODE_HYBRID {
            /* Shrink packet to what the encoder actually used. */
            nb_compr_bytes = ret;
            enc_sync_in(&mut enc, data);
            ec_enc_shrink(&mut enc, nb_compr_bytes as u32);
            enc_sync_out(&enc, data);
        }
        /* NOTE: We could speed this up slightly (at the expense of code size) by just adding a function that prefills the buffer */
        let off = (st.channels * (frame_size - N2 - N4)) as usize;
        celt_encode_with_ec(&mut st.celt_enc, &pcm_buf[off..], N4, &mut dummy, 2, None);

        let off = (st.channels * (frame_size - N2)) as usize;
        let err = celt_encode_with_ec(
            &mut st.celt_enc,
            &pcm_buf[off..],
            N2,
            &mut data[1 + nb_compr_bytes as usize..],
            redundancy_bytes,
            None,
        );
        if err < 0 {
            return OPUS_INTERNAL_ERROR;
        }
        let mut v = 0;
        celt_encoder_ctl(&mut st.celt_enc, OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        redundant_rng = v as opus_uint32;
    }

    /* Signalling the mode in the first byte */
    data[0] |= gen_toc(st.mode, st.Fs / frame_size, curr_bandwidth, st.stream_channels);

    st.rangeFinal ^= redundant_rng;

    if to_celt != 0 {
        st.prev_mode = MODE_CELT_ONLY;
    } else {
        st.prev_mode = st.mode;
    }
    st.prev_channels = st.stream_channels;
    st.prev_framesize = frame_size;

    st.first = 0;

    /* DTX decision */
    if st.use_dtx != 0 && st.silk_mode.useDTX == 0 {
        if decide_dtx_mode(activity, &mut st.nb_no_activity_ms_Q1, 2 * 1000 * frame_size / st.Fs) != 0
        {
            st.rangeFinal = 0;
            data[0] = gen_toc(st.mode, st.Fs / frame_size, curr_bandwidth, st.stream_channels);
            return 1;
        }
    } else {
        st.nb_no_activity_ms_Q1 = 0;
    }

    /* In the unlikely case that the SILK encoder busted its target, tell
       the decoder to call the PLC */
    if enc.ec_tell() > (max_data_bytes - 1) * 8 {
        if max_data_bytes < 2 {
            return OPUS_BUFFER_TOO_SMALL;
        }
        data[1] = 0;
        ret = 1;
        st.rangeFinal = 0;
    } else if st.mode == MODE_SILK_ONLY && redundancy == 0 {
        /*When in LPC only mode it's perfectly
          reasonable to strip off trailing zero bytes as
          the required range decoder behavior is to
          fill these in. This can't be done when the MDCT
          modes are used because the decoder needs to know
          the actual length for allocation purposes.*/
        while ret > 2 && data[ret as usize] == 0 {
            ret -= 1;
        }
    }
    /* Count ToC and redundancy */
    ret += 1 + redundancy_bytes;
    apply_padding = (!(st.use_vbr != 0)) as i32;
    if st.dred_duration > 0 && st.dred_encoder.loaded != 0 && first_frame != 0 {
        let mut buf = [0u8; DRED_MAX_DATA_SIZE as usize];
        let mut dred_chunks = IMIN((st.dred_duration + 5) / 4, DRED_NUM_REDUNDANCY_FRAMES / 2);
        if st.use_vbr != 0 {
            dred_chunks = IMIN(dred_chunks, st.dred_target_chunks);
        }
        /* Remaining space for DRED, accounting for cost the 3 extra bytes for code 3, padding length, and extension number. */
        let mut dred_bytes_left = IMIN(DRED_MAX_DATA_SIZE, orig_max_data_bytes - ret - 3);
        /* Account for the extra bytes required to signal large padding length. */
        dred_bytes_left -= (dred_bytes_left + 1 + DRED_EXPERIMENTAL_BYTES) / 255;
        /* Check whether we actually have something to encode. */
        if dred_chunks >= 1 && dred_bytes_left >= DRED_MIN_BYTES + DRED_EXPERIMENTAL_BYTES {
            let dred_bytes = dred_encode_silk_frame(
                &mut st.dred_encoder,
                &mut buf[DRED_EXPERIMENTAL_BYTES as usize..],
                dred_chunks,
                dred_bytes_left - DRED_EXPERIMENTAL_BYTES,
                st.dred_q0,
                st.dred_dQ,
                st.dred_qmax,
                &st.activity_mem,
                st.arch,
            );
            if dred_bytes > 0 {
                let dred_bytes = dred_bytes + DRED_EXPERIMENTAL_BYTES;
                debug_assert!(dred_bytes <= dred_bytes_left);
                let extension = crate::opus::packet::OpusExtensionData {
                    id: DRED_EXTENSION_ID,
                    frame: 0,
                    data: buf[..dred_bytes as usize].to_vec(),
                    len: dred_bytes,
                };
                ret = opus_packet_pad_impl(
                    data,
                    ret,
                    orig_max_data_bytes,
                    (!(st.use_vbr != 0)) as i32,
                    &[extension],
                );
                if ret < 0 {
                    return OPUS_INTERNAL_ERROR;
                }
                apply_padding = 0;
            }
        }
    }
    if apply_padding != 0 {
        if opus_packet_pad(data, ret, orig_max_data_bytes) != OPUS_OK {
            return OPUS_INTERNAL_ERROR;
        }
        ret = orig_max_data_bytes;
    }
    ret
}

/* ------------------------------------------------------------------------- *
 * opus_encode / opus_encode24 / opus_encode_float
 * ------------------------------------------------------------------------- */

/// `opus_encode()` (`src/opus_encoder.c:2871`, float-build `#else` branch).
pub fn opus_encode(
    st: &mut OpusEncoder,
    pcm: &[opus_int16],
    analysis_frame_size: i32,
    data: &mut [u8],
    max_data_bytes: opus_int32,
) -> i32 {
    let frame_size = frame_size_select(st.application, analysis_frame_size, st.variable_duration, st.Fs);
    if frame_size <= 0 {
        return OPUS_BAD_ARG;
    }
    let mut in_ = vec![0.0f32 as opus_res; (frame_size * st.channels) as usize];

    for i in 0..(frame_size * st.channels) as usize {
        in_[i] = INT16TORES(pcm[i]);
    }
    opus_encode_native(
        st,
        &in_,
        frame_size,
        data,
        max_data_bytes,
        16,
        Some(analysis::AnalysisPcm::Int(pcm)),
        analysis_frame_size,
        0,
        -2,
        st.channels,
        1,
    )
}

/// `opus_encode24()` (`src/opus_encoder.c:2897`, float-build `#else` branch).
pub fn opus_encode24(
    st: &mut OpusEncoder,
    pcm: &[opus_int32],
    analysis_frame_size: i32,
    data: &mut [u8],
    max_data_bytes: opus_int32,
) -> i32 {
    let frame_size = frame_size_select(st.application, analysis_frame_size, st.variable_duration, st.Fs);
    if frame_size <= 0 {
        return OPUS_BAD_ARG;
    }
    let mut in_ = vec![0.0f32 as opus_res; (frame_size * st.channels) as usize];

    for i in 0..(frame_size * st.channels) as usize {
        in_[i] = INT24TORES(pcm[i]);
    }
    opus_encode_native(
        st,
        &in_,
        frame_size,
        data,
        max_data_bytes,
        MAX_ENCODING_DEPTH,
        Some(analysis::AnalysisPcm::Int24(pcm)),
        analysis_frame_size,
        0,
        -2,
        st.channels,
        1,
    )
}

/// `opus_encode_float()` (`src/opus_encoder.c:2930`, float-build branch).
pub fn opus_encode_float(
    st: &mut OpusEncoder,
    pcm: &[f32],
    analysis_frame_size: i32,
    data: &mut [u8],
    out_data_bytes: opus_int32,
) -> i32 {
    let frame_size = frame_size_select(st.application, analysis_frame_size, st.variable_duration, st.Fs);
    opus_encode_native(
        st,
        pcm,
        frame_size,
        data,
        out_data_bytes,
        MAX_ENCODING_DEPTH,
        Some(analysis::AnalysisPcm::Float(pcm)),
        analysis_frame_size,
        0,
        -2,
        st.channels,
        1,
    )
}

/* ------------------------------------------------------------------------- *
 * OpusEncoder: the contract of docs/internal-api.md
 * ------------------------------------------------------------------------- */

impl OpusEncoder {
    /// `opus_encoder_get_size()` (`src/opus_encoder.c:179`).
    pub fn get_size(channels: i32) -> i32 {
        /* C: `opus_encoder_init(NULL, 48000, channels, OPUS_APPLICATION_AUDIO)`. */
        if channels != 1 && channels != 2 {
            return 0;
        }
        let silk = if channels == 1 {
            SILK_ENCODER_SIZE_1
        } else {
            SILK_ENCODER_SIZE_2
        };
        let mut base_size = align(OPUS_ENCODER_STRUCT_SIZE);
        if channels == 1 {
            base_size = align(base_size - MAX_ENCODER_BUFFER * 4);
        }
        let celt = celt_encoder_get_size(channels);
        base_size + silk + celt
    }

    /// `opus_encoder_init(NULL, Fs, channels, application)` — the C idiom (used
    /// by `src/opus_multistream_encoder.c`) for validating the arguments and
    /// obtaining the allocation size without creating a state.
    ///
    /// Unlike [`OpusEncoder::get_size`] (which is `opus_encoder_get_size()` and
    /// always passes `48000`/`OPUS_APPLICATION_AUDIO`), this depends on both
    /// `Fs` and `application`: `OPUS_APPLICATION_RESTRICTED_SILK` drops the CELT
    /// size, `OPUS_APPLICATION_RESTRICTED_CELT` drops the SILK size, and both
    /// shrink the delay buffer.  Negative return values are the `OPUS_*` errors
    /// `opus_encoder_init()` can produce.
    pub fn opus_encoder_init_size(Fs: opus_int32, channels: i32, application: i32) -> i32 {
        if !(Fs == 48000 || Fs == 24000 || Fs == 16000 || Fs == 12000 || Fs == 8000)
            || (channels != 1 && channels != 2)
            || (application != OPUS_APPLICATION_VOIP
                && application != OPUS_APPLICATION_AUDIO
                && application != OPUS_APPLICATION_RESTRICTED_LOWDELAY
                && application != OPUS_APPLICATION_RESTRICTED_SILK
                && application != OPUS_APPLICATION_RESTRICTED_CELT)
        {
            return OPUS_BAD_ARG;
        }
        let mut silkEncSizeBytes = if channels == 1 {
            SILK_ENCODER_SIZE_1
        } else {
            SILK_ENCODER_SIZE_2
        };
        silkEncSizeBytes = align(silkEncSizeBytes);
        if application == OPUS_APPLICATION_RESTRICTED_CELT {
            silkEncSizeBytes = 0;
        }
        let celtEncSizeBytes = if application != OPUS_APPLICATION_RESTRICTED_SILK {
            celt_encoder_get_size(channels)
        } else {
            0
        };
        let mut base_size = align(OPUS_ENCODER_STRUCT_SIZE);
        if application == OPUS_APPLICATION_RESTRICTED_SILK
            || application == OPUS_APPLICATION_RESTRICTED_CELT
        {
            base_size = align(base_size - MAX_ENCODER_BUFFER * 2 * 4);
        } else if channels == 1 {
            base_size = align(base_size - MAX_ENCODER_BUFFER * 4);
        }
        base_size + silkEncSizeBytes + celtEncSizeBytes
    }

    /// `opus_encoder_init()` (`src/opus_encoder.c:190`).
    pub fn init(&mut self, Fs: opus_int32, channels: i32, application: i32) -> i32 {
        if !(Fs == 48000 || Fs == 24000 || Fs == 16000 || Fs == 12000 || Fs == 8000)
            || (channels != 1 && channels != 2)
            || (application != OPUS_APPLICATION_VOIP
                && application != OPUS_APPLICATION_AUDIO
                && application != OPUS_APPLICATION_RESTRICTED_LOWDELAY
                && application != OPUS_APPLICATION_RESTRICTED_SILK
                && application != OPUS_APPLICATION_RESTRICTED_CELT)
        {
            return OPUS_BAD_ARG;
        }

        /* `OPUS_CLEAR((char*)st, tot_size)`: the whole allocation (both
           sub-encoders included) is zeroed. */
        *self = OpusEncoder::default();

        /* Create SILK encoder */
        let mut silkEncSizeBytes: opus_int = 0;
        let ret = silk_Get_Encoder_Size(&mut silkEncSizeBytes, channels);
        if ret != 0 {
            return OPUS_BAD_ARG;
        }

        self.stream_channels = channels;
        self.channels = channels;

        self.Fs = Fs;

        self.arch = opus_select_arch();

        if application != OPUS_APPLICATION_RESTRICTED_CELT {
            let ret =
                silk_InitEncoder(&mut self.silk_enc, channels, self.arch, &mut self.silk_mode);
            if ret != 0 {
                return OPUS_INTERNAL_ERROR;
            }
        }

        /* default SILK parameters */
        self.silk_mode.nChannelsAPI = channels;
        self.silk_mode.nChannelsInternal = channels;
        self.silk_mode.API_sampleRate = self.Fs;
        self.silk_mode.maxInternalSampleRate = 16000;
        self.silk_mode.minInternalSampleRate = 8000;
        self.silk_mode.desiredInternalSampleRate = 16000;
        self.silk_mode.payloadSize_ms = 20;
        self.silk_mode.bitRate = 25000;
        self.silk_mode.packetLossPercentage = 0;
        self.silk_mode.complexity = 9;
        self.silk_mode.useInBandFEC = 0;
        self.silk_mode.useDRED = 0;
        self.silk_mode.useDTX = 0;
        self.silk_mode.useCBR = 0;
        self.silk_mode.reducedDependency = 0;

        /* Create CELT encoder */
        /* Initialize CELT encoder */
        if application != OPUS_APPLICATION_RESTRICTED_SILK {
            let err = celt_encoder_init(&mut self.celt_enc, Fs, channels, self.arch);
            if err != OPUS_OK {
                return OPUS_INTERNAL_ERROR;
            }
            let mut v = 0;
            celt_encoder_ctl(&mut self.celt_enc, CELT_SET_SIGNALLING_REQUEST, &mut v);
            let mut v = self.silk_mode.complexity;
            celt_encoder_ctl(&mut self.celt_enc, OPUS_SET_COMPLEXITY_REQUEST, &mut v);
        }

        /* Initialize DRED Encoder */
        dred_encoder_init(&mut self.dred_encoder, Fs, channels);

        self.use_vbr = 1;
        /* Makes constrained VBR the default (safer for real-time use) */
        self.vbr_constraint = 1;
        self.user_bitrate_bps = OPUS_AUTO;
        self.bitrate_bps = 3000 + Fs * channels;
        self.application = application;
        self.signal_type = OPUS_AUTO;
        self.user_bandwidth = OPUS_AUTO;
        self.max_bandwidth = OPUS_BANDWIDTH_FULLBAND;
        self.force_channels = OPUS_AUTO;
        self.user_forced_mode = OPUS_AUTO;
        self.voice_ratio = -1;
        if application != OPUS_APPLICATION_RESTRICTED_CELT
            && application != OPUS_APPLICATION_RESTRICTED_SILK
        {
            self.encoder_buffer = self.Fs / 100;
        } else {
            self.encoder_buffer = 0;
        }
        self.lsb_depth = 24;
        self.variable_duration = OPUS_FRAMESIZE_ARG;

        /* Delay compensation of 4 ms (2.5 ms for SILK's extra look-ahead
           + 1.5 ms for SILK resamplers and stereo prediction) */
        self.delay_compensation = self.Fs / 250;

        self.hybrid_stereo_width_Q14 = (1 << 14) as i16;
        self.prev_HB_gain = Q15ONE;
        self.variable_HP_smth2_Q15 = silk_LSHIFT(silk_lin2log(VARIABLE_HP_MIN_CUTOFF_HZ), 8);
        self.first = 1;
        self.mode = MODE_HYBRID;
        self.bandwidth = OPUS_BANDWIDTH_FULLBAND;

        analysis::tonality_analysis_init(&mut self.analysis, self.Fs);        self.analysis.application = self.application;

        OPUS_OK
    }

    /// `opus_encoder_create()` (`src/opus_encoder.c:628`).
    ///
    /// The C distinguishes "bad argument" from "allocation failed"; Rust's
    /// `Box` cannot fail, so `OPUS_ALLOC_FAIL` is unreachable here.
    pub fn create(fs: i32, channels: i32, application: i32, error: &mut i32) -> Option<Box<OpusEncoder>> {
        if !(fs == 48000 || fs == 24000 || fs == 16000 || fs == 12000 || fs == 8000)
            || (channels != 1 && channels != 2)
            || (application != OPUS_APPLICATION_VOIP
                && application != OPUS_APPLICATION_AUDIO
                && application != OPUS_APPLICATION_RESTRICTED_LOWDELAY
                && application != OPUS_APPLICATION_RESTRICTED_SILK
                && application != OPUS_APPLICATION_RESTRICTED_CELT)
        {
            *error = OPUS_BAD_ARG;
            return None;
        }
        let mut st = Box::new(OpusEncoder::default());
        let ret = st.init(fs, channels, application);
        *error = ret;
        if ret != OPUS_OK {
            return None;
        }
        Some(st)
    }

    /// `opus_encoder_destroy()` (`src/opus_encoder.c:3360`): the owned `Box`
    /// makes this a no-op, kept for contract parity.
    pub fn destroy(self) {}

    /// `opus_encoder_ctl()` (`src/opus_encoder.c:2963`).
    ///
    /// `value` is an in/out integer: GET requests write their result into it,
    /// SET requests read the value from it (this is how the variadic C function
    /// is exposed; see `docs/internal-api.md`).  The three requests whose C
    /// argument is a *pointer* (`OPUS_SET_ENERGY_MASK`, `CELT_GET_MODE`,
    /// `OPUS_SET_DNN_BLOB`) cannot be expressed this way and return
    /// `OPUS_UNIMPLEMENTED`; `set_energy_mask()`/`get_mode()` provide the first
    /// two.
    pub fn ctl(&mut self, request: i32, value: &mut i32) -> i32 {
        match request {
            OPUS_SET_APPLICATION_REQUEST => {
                if self.application == OPUS_APPLICATION_RESTRICTED_SILK
                    || self.application == OPUS_APPLICATION_RESTRICTED_CELT
                {
                    return OPUS_BAD_ARG;
                }
                if (*value != OPUS_APPLICATION_VOIP
                    && *value != OPUS_APPLICATION_AUDIO
                    && *value != OPUS_APPLICATION_RESTRICTED_LOWDELAY)
                    || (self.first == 0 && self.application != *value)
                {
                    return OPUS_BAD_ARG;
                }
                self.application = *value;
                self.analysis.application = *value;
            }
            OPUS_GET_APPLICATION_REQUEST => {
                *value = self.application;
            }
            OPUS_SET_BITRATE_REQUEST => {
                if *value != OPUS_AUTO && *value != OPUS_BITRATE_MAX {
                    if *value <= 0 {
                        return OPUS_BAD_ARG;
                    } else if *value <= 500 {
                        *value = 500;
                    } else if *value > 750000 * self.channels {
                        *value = 750000 * self.channels;
                    }
                }
                self.user_bitrate_bps = *value;
            }
            OPUS_GET_BITRATE_REQUEST => {
                *value = user_bitrate_to_bitrate(self, self.prev_framesize, 1276);
            }
            OPUS_SET_FORCE_CHANNELS_REQUEST => {
                if (*value < 1 || *value > self.channels) && *value != OPUS_AUTO {
                    return OPUS_BAD_ARG;
                }
                self.force_channels = *value;
            }
            OPUS_GET_FORCE_CHANNELS_REQUEST => {
                *value = self.force_channels;
            }
            OPUS_SET_MAX_BANDWIDTH_REQUEST => {
                if *value < OPUS_BANDWIDTH_NARROWBAND || *value > OPUS_BANDWIDTH_FULLBAND {
                    return OPUS_BAD_ARG;
                }
                self.max_bandwidth = *value;
                if self.max_bandwidth == OPUS_BANDWIDTH_NARROWBAND {
                    self.silk_mode.maxInternalSampleRate = 8000;
                } else if self.max_bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
                    self.silk_mode.maxInternalSampleRate = 12000;
                } else {
                    self.silk_mode.maxInternalSampleRate = 16000;
                }
            }
            OPUS_GET_MAX_BANDWIDTH_REQUEST => {
                *value = self.max_bandwidth;
            }
            OPUS_SET_BANDWIDTH_REQUEST => {
                if (*value < OPUS_BANDWIDTH_NARROWBAND || *value > OPUS_BANDWIDTH_FULLBAND)
                    && *value != OPUS_AUTO
                {
                    return OPUS_BAD_ARG;
                }
                self.user_bandwidth = *value;
                if self.user_bandwidth == OPUS_BANDWIDTH_NARROWBAND {
                    self.silk_mode.maxInternalSampleRate = 8000;
                } else if self.user_bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
                    self.silk_mode.maxInternalSampleRate = 12000;
                } else {
                    self.silk_mode.maxInternalSampleRate = 16000;
                }
            }
            OPUS_GET_BANDWIDTH_REQUEST => {
                *value = self.bandwidth;
            }
            OPUS_SET_DTX_REQUEST => {
                if *value < 0 || *value > 1 {
                    return OPUS_BAD_ARG;
                }
                self.use_dtx = *value;
            }
            OPUS_GET_DTX_REQUEST => {
                *value = self.use_dtx;
            }
            OPUS_SET_COMPLEXITY_REQUEST => {
                if *value < 0 || *value > 10 {
                    return OPUS_BAD_ARG;
                }
                self.silk_mode.complexity = *value;
                if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
                    celt_encoder_ctl(&mut self.celt_enc, OPUS_SET_COMPLEXITY_REQUEST, value);
                }
            }
            OPUS_GET_COMPLEXITY_REQUEST => {
                *value = self.silk_mode.complexity;
            }
            OPUS_SET_INBAND_FEC_REQUEST => {
                if *value < 0 || *value > 2 {
                    return OPUS_BAD_ARG;
                }
                self.fec_config = *value;
                self.silk_mode.useInBandFEC = (*value != 0) as i32;
            }
            OPUS_GET_INBAND_FEC_REQUEST => {
                *value = self.fec_config;
            }
            OPUS_SET_PACKET_LOSS_PERC_REQUEST => {
                if *value < 0 || *value > 100 {
                    return OPUS_BAD_ARG;
                }
                self.silk_mode.packetLossPercentage = *value;
                if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
                    celt_encoder_ctl(&mut self.celt_enc, OPUS_SET_PACKET_LOSS_PERC_REQUEST, value);
                }
            }
            OPUS_GET_PACKET_LOSS_PERC_REQUEST => {
                *value = self.silk_mode.packetLossPercentage;
            }
            OPUS_SET_VBR_REQUEST => {
                if *value < 0 || *value > 1 {
                    return OPUS_BAD_ARG;
                }
                self.use_vbr = *value;
                self.silk_mode.useCBR = 1 - *value;
            }
            OPUS_GET_VBR_REQUEST => {
                *value = self.use_vbr;
            }
            OPUS_SET_VOICE_RATIO_REQUEST => {
                if *value < -1 || *value > 100 {
                    return OPUS_BAD_ARG;
                }
                self.voice_ratio = *value;
            }
            OPUS_GET_VOICE_RATIO_REQUEST => {
                *value = self.voice_ratio;
            }
            OPUS_SET_VBR_CONSTRAINT_REQUEST => {
                if *value < 0 || *value > 1 {
                    return OPUS_BAD_ARG;
                }
                self.vbr_constraint = *value;
            }
            OPUS_GET_VBR_CONSTRAINT_REQUEST => {
                *value = self.vbr_constraint;
            }
            OPUS_SET_SIGNAL_REQUEST => {
                if *value != OPUS_AUTO && *value != OPUS_SIGNAL_VOICE && *value != OPUS_SIGNAL_MUSIC {
                    return OPUS_BAD_ARG;
                }
                self.signal_type = *value;
            }
            OPUS_GET_SIGNAL_REQUEST => {
                *value = self.signal_type;
            }
            OPUS_GET_LOOKAHEAD_REQUEST => {
                *value = self.Fs / 400;
                if self.application != OPUS_APPLICATION_RESTRICTED_LOWDELAY
                    && self.application != OPUS_APPLICATION_RESTRICTED_CELT
                {
                    *value += self.delay_compensation;
                }
            }
            OPUS_GET_SAMPLE_RATE_REQUEST => {
                *value = self.Fs;
            }
            OPUS_GET_FINAL_RANGE_REQUEST => {
                *value = self.rangeFinal as i32;
            }
            OPUS_SET_LSB_DEPTH_REQUEST => {
                if *value < 8 || *value > 24 {
                    return OPUS_BAD_ARG;
                }
                self.lsb_depth = *value;
            }
            OPUS_GET_LSB_DEPTH_REQUEST => {
                *value = self.lsb_depth;
            }
            OPUS_SET_EXPERT_FRAME_DURATION_REQUEST => {
                if *value != OPUS_FRAMESIZE_ARG
                    && *value != OPUS_FRAMESIZE_2_5_MS
                    && *value != OPUS_FRAMESIZE_5_MS
                    && *value != OPUS_FRAMESIZE_10_MS
                    && *value != OPUS_FRAMESIZE_20_MS
                    && *value != OPUS_FRAMESIZE_40_MS
                    && *value != OPUS_FRAMESIZE_60_MS
                    && *value != OPUS_FRAMESIZE_80_MS
                    && *value != OPUS_FRAMESIZE_100_MS
                    && *value != OPUS_FRAMESIZE_120_MS
                {
                    return OPUS_BAD_ARG;
                }
                self.variable_duration = *value;
            }
            OPUS_GET_EXPERT_FRAME_DURATION_REQUEST => {
                *value = self.variable_duration;
            }
            OPUS_SET_PREDICTION_DISABLED_REQUEST => {
                if *value > 1 || *value < 0 {
                    return OPUS_BAD_ARG;
                }
                self.silk_mode.reducedDependency = *value;
            }
            OPUS_GET_PREDICTION_DISABLED_REQUEST => {
                *value = self.silk_mode.reducedDependency;
            }
            OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST => {
                if *value < 0 || *value > 1 {
                    return OPUS_BAD_ARG;
                }
                if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
                    celt_encoder_ctl(
                        &mut self.celt_enc,
                        OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST,
                        value,
                    );
                }
            }
            OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST => {
                if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
                    celt_encoder_ctl(
                        &mut self.celt_enc,
                        OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST,
                        value,
                    );
                } else {
                    *value = 0;
                }
            }
            OPUS_SET_DRED_DURATION_REQUEST => {
                if *value < 0 || *value > DRED_MAX_FRAMES {
                    return OPUS_BAD_ARG;
                }
                self.dred_duration = *value;
                self.silk_mode.useDRED = (*value != 0) as i32;
            }
            OPUS_GET_DRED_DURATION_REQUEST => {
                *value = self.dred_duration;
            }
            OPUS_RESET_STATE => {
                /* `OPUS_CLEAR((char*)&st->OPUS_ENCODER_RESET_START, ...)` */
                self.stream_channels = 0;
                self.hybrid_stereo_width_Q14 = 0;
                self.variable_HP_smth2_Q15 = 0;
                self.prev_HB_gain = 0.0;
                self.hp_mem = [0.0; 4];
                self.mode = 0;
                self.prev_mode = 0;
                self.prev_channels = 0;
                self.prev_framesize = 0;
                self.bandwidth = 0;
                self.auto_bandwidth = 0;
                self.silk_bw_switch = 0;
                self.first = 0;
                self.energy_masking = None;
                self.width_mem = StereoWidthState::default();
                self.detected_bandwidth = 0;
                self.nb_no_activity_ms_Q1 = 0;
                self.peak_signal_energy = 0.0;
                self.dred_duration = 0;
                self.dred_q0 = 0;
                self.dred_dQ = 0;
                self.dred_qmax = 0;
                self.dred_target_chunks = 0;
                self.activity_mem = [0; (DRED_MAX_FRAMES * 4) as usize];
                self.nonfinal_frame = 0;
                self.rangeFinal = 0;
                self.delay_buffer = [0.0; (MAX_ENCODER_BUFFER * 2) as usize];

                self.analysis.tonality_analysis_reset();

                if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
                    let mut v = 0;
                    celt_encoder_ctl(&mut self.celt_enc, OPUS_RESET_STATE, &mut v);
                }
                if self.application != OPUS_APPLICATION_RESTRICTED_CELT {
                    let mut dummy = silk_EncControlStruct::default();
                    silk_InitEncoder(&mut self.silk_enc, self.channels, self.arch, &mut dummy);
                }
                dred_encoder_reset(&mut self.dred_encoder);
                self.stream_channels = self.channels;
                self.hybrid_stereo_width_Q14 = (1 << 14) as i16;
                self.prev_HB_gain = Q15ONE;
                self.first = 1;
                self.mode = MODE_HYBRID;
                self.bandwidth = OPUS_BANDWIDTH_FULLBAND;
                self.variable_HP_smth2_Q15 = silk_LSHIFT(silk_lin2log(VARIABLE_HP_MIN_CUTOFF_HZ), 8);
            }
            OPUS_SET_FORCE_MODE_REQUEST => {
                if (*value < MODE_SILK_ONLY || *value > MODE_CELT_ONLY) && *value != OPUS_AUTO {
                    return OPUS_BAD_ARG;
                }
                self.user_forced_mode = *value;
            }
            OPUS_SET_LFE_REQUEST => {
                self.lfe = *value;
                if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
                    celt_encoder_ctl(&mut self.celt_enc, OPUS_SET_LFE_REQUEST, value);
                }
            }
            OPUS_GET_IN_DTX_REQUEST => {
                if self.silk_mode.useDTX != 0
                    && (self.prev_mode == MODE_SILK_ONLY || self.prev_mode == MODE_HYBRID)
                {
                    /* DTX determined by Silk. */
                    *value = (self.silk_enc.state_Fxx[0].sCmn.noSpeechCounter
                        >= NB_SPEECH_FRAMES_BEFORE_DTX) as i32;
                    /* Stereo: check second channel unless only the middle channel was encoded. */
                    if *value == 1
                        && self.silk_mode.nChannelsInternal == 2
                        && self.silk_enc.prev_decode_only_middle == 0
                    {
                        *value = (self.silk_enc.state_Fxx[1].sCmn.noSpeechCounter
                            >= NB_SPEECH_FRAMES_BEFORE_DTX)
                            as i32;
                    }
                } else if self.use_dtx != 0 {
                    /* DTX determined by Opus. */
                    *value =
                        (self.nb_no_activity_ms_Q1 >= NB_SPEECH_FRAMES_BEFORE_DTX * 20 * 2) as i32;
                } else {
                    *value = 0;
                }
            }
            _ => {
                return OPUS_UNIMPLEMENTED;
            }
        }
        OPUS_OK
    }

    /// `opus_encoder_ctl(st, OPUS_SET_ENERGY_MASK_REQUEST, mask)`
    /// (`src/opus_encoder.c:3260`).  The C request carries a pointer, which the
    /// integer `ctl()` interface cannot express; this is the typed equivalent.
    pub fn set_energy_mask(&mut self, mask: Option<&[celt_glog]>) -> i32 {
        self.energy_masking = mask.map(|m| m.to_vec());
        if self.application != OPUS_APPLICATION_RESTRICTED_SILK {
            opus_custom_encoder_ctl_energy_mask(&mut self.celt_enc, mask);
        }
        OPUS_OK
    }

    /// `opus_encoder_ctl(st, CELT_GET_MODE_REQUEST, &mode)`.
    pub fn get_mode(&self) -> &CELTMode {
        &self.celt_enc.mode
    }

    /// `opus_encode()`.
    pub fn encode(&mut self, pcm: &[i16], frame_size: i32, data: &mut [u8], max_data_bytes: i32) -> i32 {
        opus_encode(self, pcm, frame_size, data, max_data_bytes)
    }

    /// `opus_encode_float()`.
    pub fn encode_float(
        &mut self,
        pcm: &[f32],
        frame_size: i32,
        data: &mut [u8],
        max_data_bytes: i32,
    ) -> i32 {
        opus_encode_float(self, pcm, frame_size, data, max_data_bytes)
    }
}

/* ========================================================================= *
 * `analysis` sub-module
 *
 * Transliteration of `src/analysis.c` + `src/mlp.c` + `src/mlp_data.c`.
 * `src/analysis.rs` does not exist in this port and this module may not edit
 * other files, so it lives here under the C names.
 * ========================================================================= */
pub mod analysis {
    use super::*;
    use crate::celt::kiss_fft::{kiss_fft_cpx, opus_fft};

    /* `src/analysis.h` */
    pub const NB_FRAMES: i32 = 8;
    pub const NB_TBANDS: i32 = 18;
    pub const ANALYSIS_BUF_SIZE: i32 = 720; /* 30 ms at 24 kHz */
    pub const ANALYSIS_COUNT_MAX: i32 = 10000;
    pub const DETECT_SIZE: i32 = 100;

    /* `src/analysis.c` */
    const TRANSITION_PENALTY: f32 = 10.0;
    const NB_TONAL_SKIP_BANDS: i32 = 9;
    const LEAKAGE_OFFSET: f32 = 2.5;
    const LEAKAGE_SLOPE: f32 = 2.0;

    /// `SCALE_ENER(e)` (float build).
    #[inline(always)]
    fn SCALE_ENER(e: f32) -> f32 {
        (1.0f32 / 32768.0 / 32768.0) * e
    }

    /// `M_PI` as `math.h` defines it (the C source's `#ifndef M_PI` fallback is
    /// not taken because `mathops.h` pulls in `<math.h>` first).
    const M_PI: f64 = core::f64::consts::PI;

    /* --------------------------------------------------------------------- *
     * `src/analysis.c` constant tables (generated from the C source)
     * --------------------------------------------------------------------- */
/// `dct_table` (src/analysis.c)
#[rustfmt::skip]
static dct_table: [f32; 128] = [
    0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.250000,
    0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.250000, 0.351851, 0.338330,
    0.311806, 0.273300, 0.224292, 0.166664, 0.102631, 0.034654, -0.034654, -0.102631, -0.166664,
    -0.224292, -0.273300, -0.311806, -0.338330, -0.351851, 0.346760, 0.293969, 0.196424,
    0.068975, -0.068975, -0.196424, -0.293969, -0.346760, -0.346760, -0.293969, -0.196424,
    -0.068975, 0.068975, 0.196424, 0.293969, 0.346760, 0.338330, 0.224292, 0.034654, -0.166664,
    -0.311806, -0.351851, -0.273300, -0.102631, 0.102631, 0.273300, 0.351851, 0.311806,
    0.166664, -0.034654, -0.224292, -0.338330, 0.326641, 0.135299, -0.135299, -0.326641,
    -0.326641, -0.135299, 0.135299, 0.326641, 0.326641, 0.135299, -0.135299, -0.326641,
    -0.326641, -0.135299, 0.135299, 0.326641, 0.311806, 0.034654, -0.273300, -0.338330,
    -0.102631, 0.224292, 0.351851, 0.166664, -0.166664, -0.351851, -0.224292, 0.102631,
    0.338330, 0.273300, -0.034654, -0.311806, 0.293969, -0.068975, -0.346760, -0.196424,
    0.196424, 0.346760, 0.068975, -0.293969, -0.293969, 0.068975, 0.346760, 0.196424, -0.196424,
    -0.346760, -0.068975, 0.293969, 0.273300, -0.166664, -0.338330, 0.034654, 0.351851,
    0.102631, -0.311806, -0.224292, 0.224292, 0.311806, -0.102631, -0.351851, -0.034654,
    0.338330, 0.166664, -0.273300,
];
/// `analysis_window` (src/analysis.c)
#[rustfmt::skip]
static analysis_window: [f32; 240] = [
    0.000043, 0.000171, 0.000385, 0.000685, 0.001071, 0.001541, 0.002098, 0.002739, 0.003466,
    0.004278, 0.005174, 0.006156, 0.007222, 0.008373, 0.009607, 0.010926, 0.012329, 0.013815,
    0.015385, 0.017037, 0.018772, 0.020590, 0.022490, 0.024472, 0.026535, 0.028679, 0.030904,
    0.033210, 0.035595, 0.038060, 0.040604, 0.043227, 0.045928, 0.048707, 0.051564, 0.054497,
    0.057506, 0.060591, 0.063752, 0.066987, 0.070297, 0.073680, 0.077136, 0.080665, 0.084265,
    0.087937, 0.091679, 0.095492, 0.099373, 0.103323, 0.107342, 0.111427, 0.115579, 0.119797,
    0.124080, 0.128428, 0.132839, 0.137313, 0.141849, 0.146447, 0.151105, 0.155823, 0.160600,
    0.165435, 0.170327, 0.175276, 0.180280, 0.185340, 0.190453, 0.195619, 0.200838, 0.206107,
    0.211427, 0.216797, 0.222215, 0.227680, 0.233193, 0.238751, 0.244353, 0.250000, 0.255689,
    0.261421, 0.267193, 0.273005, 0.278856, 0.284744, 0.290670, 0.296632, 0.302628, 0.308658,
    0.314721, 0.320816, 0.326941, 0.333097, 0.339280, 0.345492, 0.351729, 0.357992, 0.364280,
    0.370590, 0.376923, 0.383277, 0.389651, 0.396044, 0.402455, 0.408882, 0.415325, 0.421783,
    0.428254, 0.434737, 0.441231, 0.447736, 0.454249, 0.460770, 0.467298, 0.473832, 0.480370,
    0.486912, 0.493455, 0.500000, 0.506545, 0.513088, 0.519630, 0.526168, 0.532702, 0.539230,
    0.545751, 0.552264, 0.558769, 0.565263, 0.571746, 0.578217, 0.584675, 0.591118, 0.597545,
    0.603956, 0.610349, 0.616723, 0.623077, 0.629410, 0.635720, 0.642008, 0.648271, 0.654508,
    0.660720, 0.666903, 0.673059, 0.679184, 0.685279, 0.691342, 0.697372, 0.703368, 0.709330,
    0.715256, 0.721144, 0.726995, 0.732807, 0.738579, 0.744311, 0.750000, 0.755647, 0.761249,
    0.766807, 0.772320, 0.777785, 0.783203, 0.788573, 0.793893, 0.799162, 0.804381, 0.809547,
    0.814660, 0.819720, 0.824724, 0.829673, 0.834565, 0.839400, 0.844177, 0.848895, 0.853553,
    0.858151, 0.862687, 0.867161, 0.871572, 0.875920, 0.880203, 0.884421, 0.888573, 0.892658,
    0.896677, 0.900627, 0.904508, 0.908321, 0.912063, 0.915735, 0.919335, 0.922864, 0.926320,
    0.929703, 0.933013, 0.936248, 0.939409, 0.942494, 0.945503, 0.948436, 0.951293, 0.954072,
    0.956773, 0.959396, 0.961940, 0.964405, 0.966790, 0.969096, 0.971321, 0.973465, 0.975528,
    0.977510, 0.979410, 0.981228, 0.982963, 0.984615, 0.986185, 0.987671, 0.989074, 0.990393,
    0.991627, 0.992778, 0.993844, 0.994826, 0.995722, 0.996534, 0.997261, 0.997902, 0.998459,
    0.998929, 0.999315, 0.999615, 0.999829, 0.999957, 1.000000,
];
/// `tbands` (src/analysis.c)
#[rustfmt::skip]
static tbands: [i32; 19] = [
    4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 136, 160, 192, 240,
];
/// `std_feature_bias` (src/analysis.c)
#[rustfmt::skip]
static std_feature_bias: [f32; 9] = [
    5.684947, 3.475288, 1.770634, 1.599784, 3.773215, 2.163313, 1.260756, 1.116868, 1.918795,
];
/// `layer0_weights` (src/mlp_data.c)
#[rustfmt::skip]
static layer0_weights: [i8; 800] = [
    -30, -9, 2, -12, 5, -1, 8, 9, 9, 8, -13, 18, -17, -34, -5, 17, -11, 0, -4, 10, 2, 10, 15,
    -8, 2, -1, 0, 5, 13, -3, -16, 1, -5, 3, 7, -28, -13, 6, 36, -3, 19, -60, -17, -28, 7, -11,
    -30, -7, 2, -42, -21, -3, 6, -22, 33, -9, 7, -30, 21, -14, 24, -11, -20, -18, -5, -12, 12,
    -49, -50, -49, 16, 9, -37, -1, 9, 34, -13, -31, -31, 12, 16, 44, -42, 2, -9, 8, -18, -6, 9,
    36, 19, 11, 13, 12, -21, 3, -28, -12, 3, 33, 25, -14, 11, 1, -94, -39, 18, -12, -11, -15,
    -7, 49, 52, 10, -43, 9, 57, 8, 21, -6, 14, -15, 44, -8, 7, -30, -13, -2, -9, 25, -2, -127,
    18, -11, -52, 26, -27, 27, 10, -10, 7, 43, 6, -24, 41, 10, -18, -27, 10, 17, 9, 10, -17,
    -10, 20, -6, 22, 55, 35, -80, 36, 25, -24, -36, 15, 9, -19, 88, 19, 64, -51, -35, 17, 0, -7,
    41, -16, 27, 4, 15, -1, 18, -16, 47, -39, -54, -8, 13, -25, -20, 102, -18, -5, 44, 11, -28,
    71, 2, -51, -5, 5, 2, -83, -9, -29, 8, 21, -53, 58, -37, -7, 13, 38, 9, 34, -1, -41, 21, 4,
    -24, -36, -33, -21, 32, 75, -2, 1, -68, -1, 47, -29, 32, 20, 12, -65, -87, 5, 16, -12, 24,
    40, 15, 7, 19, -26, -17, 17, 6, -2, -37, -30, -9, 32, -127, -39, 0, -31, -27, 4, -22, 23,
    -6, -77, 35, -61, 32, -37, -24, 13, -11, -1, -40, -3, 17, -7, 13, 11, 59, -19, 10, 6, -18,
    0, 13, 3, -6, -23, 19, 11, -17, 13, -1, -80, 40, -53, 69, -29, -54, 0, -4, 33, -25, -2, 38,
    35, 36, -15, 46, 2, -13, -16, -8, -8, 12, -24, -9, -55, -5, -9, 32, 11, 7, 12, -18, -10,
    -86, -38, 54, 37, -25, 18, -43, 7, -27, -27, -54, 13, 9, 22, 70, 6, 35, -7, 23, -15, -44,
    -6, 7, -66, -85, 32, 40, -19, -9, -7, 12, -15, 7, 2, 6, -35, 11, 28, 0, 26, 14, 1, 1, 4, 12,
    18, 35, 22, -18, -3, 14, -1, 7, 14, -8, -14, -3, 4, -3, -19, -7, -1, -25, -27, 25, -26, -2,
    33, -22, -27, -25, 4, -9, 7, 21, 26, -30, 10, -9, -20, 11, 27, 10, 5, -18, 14, -4, 2, -17,
    -5, -7, -9, -13, 15, 29, 1, -10, -16, -10, 35, 36, -7, -22, -44, 17, 30, 22, 21, -1, 22,
    -11, 32, -8, -7, 5, -10, 5, 30, -20, 29, -20, -34, 12, -4, -6, 6, -13, 10, -5, -68, -1, 24,
    9, 19, -24, -64, 31, 19, 27, -26, 75, -45, 41, 39, -42, 8, 6, 23, -30, 16, -25, 30, 34, 8,
    -38, -3, 18, 16, -31, 22, -4, -9, 1, 20, 9, 38, -32, 0, -45, 0, -6, -13, 11, -25, -32, -22,
    31, -24, -11, -11, -4, -4, 20, -34, 22, 20, 9, -25, 27, -5, 28, -29, 29, 6, 21, -6, -18, 54,
    4, -46, 23, 21, -14, -31, 36, -41, -24, 4, 22, 10, 11, 7, 36, -32, -13, -52, -17, 24, 28,
    -37, -36, -1, 24, 9, -38, 35, 48, 18, 2, -1, 45, 10, 39, 24, -38, 13, 8, -16, 8, 25, 11, 7,
    -29, -11, 7, 20, -30, -38, -45, 14, -18, -28, -9, 65, 61, 22, -53, -38, -16, 36, 46, 20,
    -39, 32, -61, -6, -6, -36, -33, -18, -28, 56, 101, 45, 11, -28, -23, -29, -61, 20, -47, 2,
    48, 27, -17, 1, 40, 1, 3, -51, 15, 35, 28, 22, 35, 53, -61, -29, 12, -6, -21, 10, 3, -20, 2,
    -25, 1, -6, 31, 11, -3, 1, -10, -52, 6, 126, -105, 122, 127, -128, 127, 127, -128, 127, 108,
    12, 127, 48, -128, -36, -128, 127, 127, -128, -128, 127, 89, -128, 127, -128, -128, -128,
    127, 127, -128, -128, -93, -82, 20, 125, 65, -82, 127, 38, -74, 81, 88, -88, 79, 51, -47,
    -111, -26, 14, 83, -88, -112, 24, 35, -101, 98, -99, -48, -45, 46, 83, -60, -79, 45, -20,
    -41, 9, 4, 52, 54, 93, -10, 4, 13, 3, 123, 6, 94, -111, -69, -14, -31, 10, 12, 53, -79, -11,
    -21, -2, -44, -72, 92, 65, -57, 56, -38, 127, -56, -128, 127, 127, -128, 86, 117, -75, -128,
    127, -19, -99, -112, 127, -128, 127, -48, 114, 118, -128, -128, 117, -17, -6, 121, -128,
    127, -128, 82, 54, -106, 127, 127, -33, 100, -39, -23, 18, -78, -34, -29, -1, -30, 127, -26,
    127, -128, 126, -128, 27, -23, -79, -120, -127, 127, 72, 66, 29, 7, -66, -56, -117, -128,
];
/// `layer0_bias` (src/mlp_data.c)
#[rustfmt::skip]
static layer0_bias: [i8; 32] = [
    51, -16, 1, 13, -5, -6, -16, -7, 11, -6, 106, 26, 28, -14, 21, -29, 7, 18, -18, -17, 21,
    -17, -9, 20, -25, -3, -34, 48, 11, -13, -31, -20,
];
/// `layer1_weights` (src/mlp_data.c)
#[rustfmt::skip]
static layer1_weights: [i8; 2304] = [
    22, -1, -7, 7, 29, -27, -31, -17, -13, 33, 44, -8, 11, 33, 24, 78, 15, 19, 30, -2, -24, 5,
    49, 5, 36, 29, -14, -11, -48, -33, 21, -42, -38, -12, 55, -37, 54, -8, 1, 36, 17, 0, 51, 31,
    59, 7, -12, 53, 4, 32, -14, 48, 5, -10, -16, -8, 1, -16, -56, -24, -6, 18, -2, 23, 6, 46,
    -6, -10, 20, 35, -44, -15, -49, 36, 16, 5, -7, -79, -67, 12, 70, -3, -79, -54, -85, -24, 47,
    -22, 33, 21, 69, -1, 11, 22, 14, -16, -16, -22, -28, -11, 11, -41, 31, -26, -33, -19, -4,
    27, 32, -50, 5, -10, -38, -22, -8, 35, -31, 1, -41, -15, -11, 44, 28, -17, -41, -23, 17, 2,
    -23, -26, -13, -13, -17, 6, 14, -31, -25, 9, -19, 39, -8, 4, 31, -1, -45, -11, -28, -92,
    -46, -15, 21, 118, -22, 45, -51, 11, -20, -20, -15, 13, -21, -97, -29, -32, -23, -42, 94, 1,
    23, -8, 63, -3, -46, 19, -26, 32, -40, -74, -26, 26, -4, -13, 30, -20, -30, -25, -14, -31,
    -45, -43, 4, -60, -48, -12, -34, 2, 2, 3, 13, 15, 11, 16, 5, 46, -9, -55, -16, -57, 29, 14,
    38, -50, -2, -44, -11, -8, 52, -27, -38, -7, 20, 47, 17, -59, 0, 47, 46, -63, 35, -17, 19,
    33, 68, -19, 2, 15, -16, 28, -16, -103, 26, -35, 47, -39, -60, 30, 31, -23, -52, -13, 116,
    47, -25, 30, 40, 30, -22, 2, 12, -27, -18, 31, -10, 27, -8, -66, 12, 14, 4, -26, -28, -13,
    3, 13, -26, -51, 37, 5, 2, -21, 47, 3, 13, 25, -41, -27, -8, -4, 5, -76, -33, 28, 10, 9,
    -46, -74, 19, 28, 25, 31, 54, -55, 68, 38, -24, -32, 2, 4, 68, 11, -1, 99, 5, 16, -2, -74,
    40, 26, -26, 33, 31, -1, -68, 14, -6, 25, 9, 29, 60, 61, 7, -7, 0, -24, 7, 77, 4, -1, 16,
    -7, 13, -15, -19, 28, -31, -24, -16, 37, 24, 13, 30, 10, -30, 11, 11, -10, 22, 60, 28, 45,
    -3, -40, -62, -5, -102, 9, -32, -27, -54, 21, 15, -5, 37, -43, -11, 37, -19, 47, -64, -128,
    -27, -114, 21, -66, 59, 46, -3, -12, -87, -9, 4, 19, -113, -36, 78, 57, -26, -38, -77, -10,
    6, 6, -75, 25, -97, -11, 33, -46, 1, 13, -21, -33, -20, 16, -6, -3, -11, -4, -27, 38, 8,
    -41, -2, -33, 18, 19, -26, 1, -29, -22, -4, -14, -55, -11, -80, -3, 11, 34, 90, 51, 11, 17,
    43, 36, 127, -32, 29, 103, 9, 27, 13, 64, 56, 70, -14, 3, -12, 10, 37, 3, 12, -22, -10, 46,
    28, 10, 20, 26, -24, 18, 9, 7, 14, 34, -5, -7, 31, -14, -56, 11, -18, -8, -17, -7, -10, -40,
    10, -33, -32, -43, 5, 9, 11, -4, 10, 50, -12, -5, 46, 9, 7, 1, 11, 15, 91, -17, 7, -50, 23,
    6, -30, -99, 0, -17, 14, 8, -10, -25, -30, -69, -62, 31, 127, 114, -23, 101, -5, -54, -6,
    -22, 7, -56, 39, 18, -29, 0, 46, 8, -79, 4, -21, 18, -32, 62, -12, -8, -12, -58, 31, -32,
    17, 6, -24, 25, 24, 9, -4, -19, 45, 6, 17, -14, 5, -27, 16, -4, -41, 25, -36, 5, 15, 12, 50,
    27, 25, 23, -44, -69, -9, -19, -48, -8, 4, 12, -6, 13, -19, -30, -36, 26, 37, -1, -3, -30,
    -42, -14, -10, -20, 26, -54, -27, -44, 4, 73, -26, 90, 32, -69, -29, -16, 3, 103, 15, -17,
    37, 24, -23, -31, 33, -37, -64, 25, 13, -81, -28, -32, 27, 5, -35, -23, 15, -22, 19, -7, 9,
    30, 19, -23, 27, -13, 43, 29, -29, -6, 9, -40, -33, -33, -32, 9, 11, -48, -8, -23, -52, 46,
    17, -22, -42, 35, -15, -41, 16, 34, 31, -42, -19, -11, 55, 7, -39, 89, -11, -33, 20, -14,
    22, 32, 3, -17, -6, 14, 34, 1, 55, -21, -90, -8, 18, 27, 13, -29, 21, 15, -33, -51, -9, -11,
    4, -16, -18, 23, -4, -4, 48, 1, 7, 29, -14, -12, -16, 17, 35, 8, 0, -7, -2, 9, 8, 17, -6,
    53, -32, -21, -50, 5, 99, -60, -5, -53, 10, -31, 12, -5, 7, 80, 36, 18, -31, 9, 98, 36, -63,
    -35, 4, -13, -28, -24, 28, -13, 18, 16, -1, -18, -34, 10, 20, 7, 4, 29, 11, 25, -7, 36, 14,
    45, 24, 1, -16, 30, 6, 35, -6, -11, -24, 13, -1, 27, 39, 20, 48, -11, -4, -13, 28, 11, -31,
    -18, 31, -29, 22, -2, -20, -16, 5, 30, -12, -28, -3, 93, -16, 23, 18, -29, 6, -54, -37, 28,
    -3, -3, -47, -3, -36, -55, -3, 41, -10, 47, -2, 23, 42, -7, -71, -27, 83, -64, 7, -24, 8,
    26, -17, 15, 12, 31, -30, -38, -13, -33, -56, 4, -17, 20, 18, 1, -30, -5, -6, -31, -14, -37,
    0, 22, 10, -30, 37, -17, 18, 6, 5, 23, -36, -32, 14, 18, -13, -61, -52, -69, 44, -30, 16,
    18, -4, -25, 14, 81, 26, -8, -23, -59, 52, -104, 17, 119, -32, 26, 17, 1, 23, 45, 29, -64,
    -57, -14, 73, 21, -13, -13, 9, -68, -7, -52, 3, 24, -39, 44, -15, 27, 14, 19, -9, -28, -11,
    5, 3, -34, -2, 2, 22, -6, -23, 4, 3, 13, -22, -13, -10, -18, 29, 6, 44, -13, -24, -8, 2, 30,
    14, 43, 6, 17, -73, -6, -7, 20, -80, -7, -7, -28, 15, -69, -38, -5, -100, -35, 15, -79, 23,
    29, -18, -27, 21, -66, -37, 8, -22, -39, 48, 4, -13, 1, -9, 11, -29, 22, 6, -49, 32, -14,
    47, -18, -4, 44, -52, -74, 43, 30, 23, -14, 5, 0, -27, 4, -7, 10, -4, 10, 1, -16, 11, -18,
    -2, -5, 2, -11, 0, -20, -4, 38, 74, 59, 39, 64, -10, 26, -3, -40, -68, 3, -30, -51, 8, -19,
    -27, -46, 51, 52, 54, 36, 90, 92, 14, 13, -5, 0, 16, -62, 16, 11, -47, -37, -6, -5, 21, 54,
    -57, 32, 42, -6, 62, -9, 16, 21, 24, 9, -10, -4, 33, 50, 13, -15, 1, -35, -48, 18, -11, -17,
    -67, -13, 21, 38, -44, 36, -16, 29, 17, 5, -10, 18, 17, -32, 2, 8, 22, -56, -15, -32, 40,
    43, 19, 46, -7, -100, -96, 19, 53, 24, 21, -26, -48, -101, -82, 61, 38, -85, -28, -34, -1,
    63, -5, -5, 39, 39, -38, 32, -12, -28, 20, 40, -8, 2, 31, 12, -35, -13, 20, -25, 30, 8, 3,
    -13, -9, -20, 2, -13, 24, 37, -10, 33, 6, 20, -16, -24, -6, -6, -19, -5, 22, 21, 10, 11, -4,
    -39, -1, 6, 49, 41, -15, -57, 21, -62, 77, -69, -13, 0, -74, 1, -7, -38, -8, 6, 63, 28, 4,
    26, -52, 82, 63, 13, 45, -33, 44, -52, -65, -21, -46, -49, 64, -17, 32, 24, 68, -39, -16,
    -5, -26, 28, 5, -61, -28, 2, 24, 11, -12, -33, 9, -37, -3, -28, 22, -37, -12, 19, 0, -18,
    -2, 14, 1, 4, 8, -9, -2, 43, -17, -2, -66, -31, 56, -40, -87, -36, -2, -4, -42, -45, -1, 31,
    -43, -15, 27, 63, -11, 32, -10, -33, 27, -19, 4, 15, -26, -34, 29, -4, -39, -65, 14, -20,
    -21, -17, -36, 13, 59, 47, -38, -33, 13, -37, -8, -37, -7, -6, -76, -31, -12, -46, 7, 24,
    -21, -30, -14, 9, 15, -12, -13, 47, -27, -25, -1, -39, 0, 20, -9, 6, 7, 4, 3, 7, 39, 50, 22,
    -7, 14, -20, 1, 70, -28, 29, -41, 10, -16, -5, -28, -2, -37, 32, -18, 17, 62, -11, -20, -50,
    36, 21, -62, -12, -56, 52, 50, 17, 3, 48, 44, -41, -25, 3, 16, -3, 0, 33, -6, 15, 27, 34,
    -25, 22, 9, 17, -11, 36, 16, -2, 12, 21, -52, 45, -2, -10, 46, 21, -18, 67, -28, -13, 30,
    37, 42, 16, -9, 11, 75, 7, -64, -40, -10, 29, 57, -23, 5, 53, -77, 3, -17, -5, 47, -55, -35,
    -36, -13, 52, -53, -71, 52, -111, -23, -26, -28, 29, -43, 55, -19, 43, -19, 54, -12, -33,
    -44, -39, -19, -10, -31, -10, 21, 38, -57, -20, 2, -25, 8, -6, 50, 12, 15, 25, -25, 15, -30,
    -6, 9, 25, 37, 19, -4, 31, -22, 2, 4, 2, 36, 7, 3, -34, -80, 36, -10, -2, -5, 31, -36, 49,
    -70, 20, -36, 21, 24, 25, -46, -51, 36, -58, -48, -40, -10, 55, 71, 47, 10, -1, 1, 2, -46,
    -68, 16, 13, 0, -74, -29, 73, -52, -18, -11, 7, -44, -82, -32, -70, -28, -1, -39, -68, -6,
    -41, 12, -22, -16, 40, -11, -25, 51, -9, 21, 4, 4, -34, 7, -78, 16, 6, -38, -30, -2, -44,
    32, 0, 22, 64, 5, -72, -2, -14, -10, -16, -8, -25, 12, 102, -58, 37, -10, -23, 15, 49, 7,
    -7, 2, -20, -32, 45, -6, 48, 28, 30, 33, -1, 22, -6, 30, 65, -17, 29, 74, 37, -26, -10, 15,
    -24, 19, -66, 22, -10, -31, -1, -18, -9, 11, 37, -4, 45, 5, 41, 17, 1, 1, 24, -58, 41, 5,
    -51, 14, 8, 43, 16, -10, -1, 45, 32, -64, 3, -33, -25, -3, -27, -68, 12, 23, -11, -13, -37,
    -40, 4, -21, -12, 32, -23, -19, 76, 41, -23, -24, -44, -65, -1, -15, 1, 71, 63, 5, 20, -3,
    21, -23, 31, -32, 18, -2, 27, 31, 46, -5, -39, -5, -35, 18, -18, -40, -10, 3, 12, 2, -2,
    -22, 40, 5, -6, 60, 36, 3, 29, -27, 10, 25, -54, 5, 26, 39, 35, -24, -37, 30, -91, 28, -4,
    -21, -27, -39, -6, 5, 12, -128, 38, -16, 29, -95, -29, 82, -2, 35, 2, 12, 8, -22, 10, 80,
    -47, 2, -25, -73, -79, 16, -30, -32, -66, 48, 21, -45, -11, -47, 14, -27, -17, -7, 15, -44,
    -14, -44, -26, -32, 26, -23, 17, -7, -28, 26, -6, 28, 6, -26, 2, 13, -14, -23, -14, 19, 46,
    16, 2, -33, -21, 28, -17, -42, 44, -37, 1, -39, 28, 84, -46, 15, 10, 13, -44, 72, -26, 26,
    32, -28, -12, -83, 2, 10, -30, -44, -10, -28, 53, 45, 65, 0, -25, 57, 36, -33, 6, 29, 44,
    -53, 11, 19, -2, -27, 35, 32, 49, 4, 23, 38, 36, 24, 10, 51, -39, 4, -7, 26, 37, -35, 11,
    -47, -18, 28, 16, -35, 42, 17, -21, -41, 28, 14, -12, 11, -45, 7, -43, -15, 18, -5, 38, -40,
    -50, -30, -21, 9, -98, 13, 12, 23, 75, -56, -7, -3, -4, -1, -34, 12, -49, 11, 26, -18, -28,
    -17, 33, 13, -14, 40, 24, -72, -37, 10, 17, -6, 22, 16, 16, -6, -12, -30, -14, 10, 40, -23,
    12, 15, -3, -15, 13, -56, -4, -30, 1, -3, -17, 27, 50, -5, 64, -36, -19, 7, 29, 22, 25, 9,
    -16, -58, -69, -40, -61, -71, -14, 42, 93, 26, 11, -6, -58, -11, 70, -52, 19, 9, -30, -33,
    11, -37, -47, -21, -22, -40, 10, 47, 4, -23, 17, 48, 41, -48, 14, 10, 15, 34, -23, -2, -47,
    23, -32, -13, -10, -26, -26, -4, 16, 38, -14, 0, -12, -7, -7, 20, 44, -1, -32, -27, -16, 4,
    -6, -18, 14, 5, 4, -29, 28, 7, -7, 15, -11, -20, -45, -36, 16, 84, 34, -59, -30, 22, 126, 8,
    68, 79, -17, 21, -68, 37, 5, 15, 63, 49, 127, -90, 85, 43, 7, 16, 9, 6, -45, -57, -43, 57,
    11, -23, -11, -29, 60, -26, 0, 7, 42, -24, 10, 23, -25, 8, -7, -40, 19, -17, 35, 4, 27, -39,
    -91, 27, -36, 34, 2, 16, -24, 25, 7, -21, 5, 17, 10, -22, -30, 9, -17, -61, -26, 33, 21, 58,
    -51, -14, 69, -38, 20, 7, 80, -4, -65, -6, -27, 53, -12, 47, -1, -15, 1, 60, 102, -79, -4,
    12, 9, 22, 37, -8, -4, 37, 2, -3, -15, -16, -11, -5, 19, -6, -43, 20, -25, -18, 10, -27, 0,
    -28, -27, -11, 10, -18, -2, -4, -16, 26, 14, -6, 7, -6, 1, 53, -2, -29, 23, 9, -30, -6, -4,
    -6, 56, 70, 0, -33, -20, -17, -9, -24, 46, -5, -105, 47, -46, -51, 20, 20, -53, -81, -1, -7,
    75, -5, -21, -65, 12, -52, 22, -50, -12, 49, 54, 76, -81, 10, 45, -41, -59, 18, -19, 25, 14,
    -31, -53, -5, 12, 31, 84, -23, 2, 7, 2, 10, -32, 39, -2, -12, 1, -9, 0, -10, -11, 9, 15, -8,
    -2, 2, -1, 10, 14, -5, -40, 19, -7, -7, 26, -4, 2, 1, -27, 35, 32, 21, -31, 26, 43, -9, 4,
    -32, 40, -62, -52, 36, 22, 38, 22, 36, -96, 6, -10, -23, -49, 15, -33, -18, -3, 0, 41, 21,
    -19, 21, 23, -39, -23, -6, 6, 47, 56, 4, 74, 0, -98, 29, -47, -14, -36, 21, -22, 22, 16, 13,
    12, 16, -5, 13, 17, -13, -15, 1, -34, -26, 26, 12, 32, 27, 13, -67, 27, 2, 8, 10, 18, 16,
    20, -17, -17, 57, -64, 5, 14, 19, 31, -18, -44, -46, -16, 4, -25, 17, -126, -24, 39, 4, 8,
    55, -25, -34, 39, -16, 3, 9, 71, 72, -31, -55, 6, 10, -25, 32, -85, -21, 18, -8, 15, 12,
    -27, -7, 1, -21, -2, -5, 48, -16, 18, 1, -22, -26, 16, 14, -31, 27, -6, -15, -21, 4, -14,
    18, -36,
];
/// `layer1_recur_weights` (src/mlp_data.c)
#[rustfmt::skip]
static layer1_recur_weights: [i8; 1728] = [
    20, 67, -99, 12, 41, -25, 49, -44, 35, 81, 110, 47, 34, -66, -14, 14, -60, 34, 29, -73, 10,
    41, 35, 89, 7, -35, 22, 7, 27, -20, -6, 56, 26, 66, 6, 33, -55, 53, 1, -21, 14, 17, 68, 55,
    59, 0, 18, -9, 5, -41, 6, -5, -114, -12, 29, 42, -23, 10, 81, -27, 20, -53, -30, -62, 40,
    95, 25, -4, 3, 18, -8, -15, -29, -82, 2, -57, -3, -61, -29, -29, 49, 2, -55, 5, -69, -99,
    -49, -51, 6, -25, 12, 89, 44, -33, 5, 41, 1, 23, -37, -37, -28, -48, 3, 4, -41, -30, -57,
    -35, -39, -1, -13, -56, -5, 50, 49, 41, -4, -4, 33, -22, -1, 33, 34, 18, 40, -42, 12, 1, -6,
    -2, 18, 17, 39, 44, 11, 65, -60, -45, 10, 91, 21, 9, -62, -11, 8, 69, 37, 24, -30, 21, 26,
    -27, 1, -28, 24, 66, -8, 6, -71, 34, 24, 44, 58, -78, -19, 57, 17, -60, 1, 12, -3, -1, -40,
    22, 11, -5, 25, 12, 1, 72, 79, 7, -50, 23, 18, 13, 21, -11, -20, 5, 77, -94, 24, 15, 57,
    -51, 3, 36, 53, -1, 4, 14, 30, -31, 22, 40, 32, -11, -34, -36, -59, 58, 25, 21, -54, -23,
    40, 46, 18, 0, 12, 54, -96, -99, -59, 5, 119, -38, 50, 55, 12, -16, 67, 0, 34, 35, 39, 35,
    -1, 69, 24, 27, -30, -35, -4, -70, 2, -44, -7, -6, 19, -9, 60, 44, -21, -10, 37, 43, -16,
    -3, 30, -15, -65, 31, -55, 18, -98, 76, 64, 25, 24, -18, -7, -68, -10, 38, 27, -60, 36, 33,
    16, 30, 34, -39, -37, 31, 12, 53, -54, 14, -26, -49, -128, -13, -5, -22, -11, -85, 55, -8,
    -51, -11, -33, -10, -31, -76, -41, 23, 44, -40, -54, -127, -101, 19, -23, -15, 15, 27, 58,
    -60, 8, 14, -33, 1, 48, -9, -11, -123, 3, 53, 23, 4, -28, 22, 2, -29, -67, 36, 12, 7, 55,
    -21, 88, 20, -1, -21, -17, 3, 41, 32, -10, -14, -5, -57, 67, 57, 21, 23, -2, -27, -73, -24,
    120, 21, 18, -35, 42, -7, 3, -45, -25, 76, -34, 50, 11, -54, -91, 3, -113, -20, -5, 47, 15,
    -47, 17, 27, -3, -26, -7, 10, 7, 74, -40, 64, -7, -5, -24, -49, -24, -3, -10, 27, -17, -8,
    -3, 14, -27, 33, 13, 39, 28, -7, -38, 29, 16, 44, 19, 55, -3, 9, -13, -57, 43, 43, 31, 0,
    -93, -17, 19, -56, 4, -12, -25, 37, -85, -13, -118, 33, -17, 56, 71, -80, -4, 6, -11, -18,
    47, -52, 25, 9, 48, -107, 1, 21, 20, -3, 10, -16, -4, 24, 17, 31, -61, -18, -50, 24, -10,
    12, 71, 26, 11, -3, 4, 1, 0, -7, -40, 18, 38, -34, 38, 17, 8, -34, 2, 21, 123, -32, -26, 43,
    14, -34, -1, -9, 37, -16, 6, -17, -62, 68, 22, 17, 11, -75, 33, -80, 62, -9, -75, 76, 36,
    -41, -8, -40, -11, -71, 40, -39, 62, -49, -81, 16, -9, -52, 52, 61, 17, -103, -27, -10, -8,
    -54, -57, 21, 23, -16, -52, 36, 18, 10, -5, 8, 15, -29, 5, -19, -37, 8, -53, 6, 19, -37, 38,
    -17, 48, 10, 0, 81, 46, 70, -29, 101, 11, 44, -44, -3, 24, 11, 3, 14, -9, 11, 14, -45, 13,
    46, -3, -57, 68, 44, 63, 98, 25, -28, -23, 15, 32, -10, 53, -6, -2, -9, -6, 16, -107, -11,
    -11, -28, 59, 57, -22, 38, 42, 83, 27, 5, 29, -30, 12, -21, -13, 31, 38, -21, 58, -10, -10,
    -15, -2, -5, 11, 12, -73, -28, -38, 22, 2, -25, 73, -52, -12, -55, 32, -63, 21, 51, 33, 52,
    -26, 55, -26, -26, 57, -32, -4, -52, -61, 21, -33, -91, -51, 69, -90, -53, -38, -44, 12,
    -76, -20, 77, -45, -7, 86, 43, -109, -33, -105, -40, -121, -10, 0, -72, 45, -51, -75, -49,
    -38, -1, -62, 18, -1, 30, -44, -14, -10, -67, 40, -10, -34, 46, -64, -32, 29, -13, 33, 3,
    -32, -5, 28, -27, -25, 93, 24, 68, -40, 57, 23, -3, -21, -58, 17, -39, -17, -22, -89, 11,
    18, -46, 27, 24, 46, 127, 61, 87, 31, 127, -36, 47, -23, 47, 127, -24, 110, 122, 30, 100, 0,
    96, -12, 6, 50, 44, -13, 73, 4, 55, -11, -15, 49, 42, -6, 20, -35, 58, 18, 38, 42, 72, 19,
    -21, 11, 9, -37, 7, 29, 31, 16, -17, 13, -50, 19, 5, -23, 51, -16, -5, 4, -24, 76, 10, -53,
    -28, -7, -65, 74, 40, -16, -29, 32, -16, -49, -35, -3, 59, -96, -50, -43, -43, -61, -15, -8,
    -36, -34, -33, -14, 11, -3, -39, 4, -114, -123, -11, -49, -21, 14, -56, 1, 43, -63, 26, 40,
    18, -10, -26, -14, -15, -35, -35, -11, 32, -44, -67, 2, 22, 7, 3, -9, -30, -51, -28, 28, 6,
    -22, 16, 34, -25, -52, -54, -8, -6, 5, 8, 20, -16, -17, -44, 27, 3, 31, -5, -48, -1, -3,
    116, 11, 71, -31, -47, 109, 50, -22, -12, -57, 32, 66, 8, -25, -93, -54, -10, 19, -76, -34,
    97, 48, -36, -18, -30, -39, -26, -12, 28, 14, 12, -12, -31, 38, 2, 10, 4, -40, 20, 16, -61,
    2, 64, 39, 5, 15, 33, 40, -61, -49, 93, -10, 33, 28, -11, -27, -18, 39, -62, -6, -6, 62, 11,
    -8, 38, -67, 12, 27, 39, -27, 123, -18, -6, -65, 83, -64, 20, 19, -11, 33, 24, 17, 56, 78,
    7, -15, 54, -101, -9, 115, -96, 50, 51, 35, 34, 27, 37, -40, -11, 8, -36, 42, -45, 2, -23,
    0, 67, -8, -9, -13, 50, -14, -27, 4, 0, -8, -14, 30, -9, 29, 15, 9, -38, 37, -8, 50, -46,
    54, 41, -11, -8, -11, -26, 39, 45, 14, -26, -17, -27, 69, 38, 39, 98, 66, 0, 42, 123, -101,
    -19, -83, 117, -32, 56, 10, 12, -88, 79, -53, 56, 63, 95, -62, 9, 36, -13, -79, -16, 37,
    -46, 35, -34, 14, 17, -54, 5, 21, -7, 7, 63, 56, 15, 27, -76, -25, 4, -26, -63, 28, -67,
    -52, 43, -47, -70, 40, -12, 40, -66, -37, 0, 35, 37, -53, 4, -17, -51, 11, 21, 14, -34, -4,
    24, -42, 29, 22, 7, 28, 12, 37, 39, -39, -19, 65, -60, -50, -2, 1, 82, 39, 19, -23, -43,
    -22, -67, -35, -34, 32, 102, 81, 127, 36, 67, -45, 1, -67, -52, -4, 35, 20, 28, 71, 86, -35,
    -9, -83, -34, 12, 9, -23, 2, 14, 28, -23, 7, -25, 45, 7, 17, -37, 0, -19, 31, 26, 40, -27,
    -16, 17, 5, -21, 23, 24, 96, -55, 52, -19, -14, -6, 1, 50, -34, 86, -53, 38, 2, -52, -36,
    -13, 60, -85, -120, 32, 7, -12, 22, 70, -7, -94, 38, -76, -31, -20, 15, -28, 7, 6, 40, 53,
    88, 3, 38, 18, -8, -22, -23, 51, 37, -9, 13, -32, 25, -21, 27, 31, 20, 18, -9, -13, 1, 21,
    -24, -13, 39, 15, -11, -29, -36, 18, 15, 8, 27, 21, -94, -1, -22, 49, 66, -1, 6, -3, -40,
    -18, 6, 28, 12, 33, -59, 62, 60, -48, 90, -1, 108, 9, 18, -2, 27, 77, -65, 82, -48, -38,
    -19, -11, 127, 50, 66, 18, -13, -22, 60, -38, 40, -14, -26, -13, 38, 67, 57, 30, 33, 26, 36,
    38, -17, 27, -28, 20, 12, -64, 18, 5, -33, -27, 13, -26, 32, 35, -5, -48, -14, 92, 43, -47,
    -14, 40, 11, 51, 66, 22, -63, -16, -61, 4, -28, 27, 20, -33, -30, -21, -29, -53, 31, -40,
    24, 43, -4, -19, 21, 67, 20, 100, -16, -93, 78, -6, -18, -52, -37, -9, 66, -31, -8, 26, 18,
    4, 24, -22, 17, -2, -13, 27, 0, 8, -18, -25, 5, -21, -24, -7, 18, -93, 21, 7, 2, -75, 69,
    50, -5, -15, -17, 60, -42, 55, 1, -4, 3, 10, 46, 16, -13, 45, -7, -10, -44, -108, 49, 2,
    -15, -64, -12, -72, 32, -38, -45, 10, -54, 13, -13, -27, -36, -64, 58, -62, -101, 88, -86,
    -71, -39, -9, -128, 32, 15, -4, 54, -16, -39, -26, -36, 46, 48, -64, -10, 19, 30, -13, 34,
    -8, 50, 60, -22, -6, -11, -30, 5, 50, 32, 56, 0, 25, 6, 68, 11, -29, 45, -9, -12, 4, 1, 18,
    -49, 0, -38, -19, 90, 29, 35, 51, 8, -48, 96, -1, -12, -9, -32, -63, -65, -7, 38, 89, 28,
    -85, -28, -23, -25, -128, 56, 79, -36, 99, -6, -37, 7, -13, -69, -46, -29, 25, 64, -21, 17,
    1, 42, -66, 1, 80, 26, -32, 21, 15, 15, 6, 6, -10, 15, 127, 5, 38, 27, 87, -57, -25, 11, 72,
    -21, -5, 11, -13, -66, 78, 36, -3, 41, -21, 8, -33, 23, 73, 28, 57, -25, -5, 4, -22, -47,
    15, 4, -57, -72, 33, 1, 18, 2, 53, -71, -99, -21, -3, -111, 108, 71, -14, 82, 25, 61, -48,
    5, 9, -51, -20, -25, -3, 14, -33, 14, -3, -34, 22, 12, -19, -38, -16, 2, 21, 16, 26, -31,
    75, 44, -31, 16, 26, 66, 17, -9, -22, -22, 22, -44, 22, 27, 2, 58, -14, 10, -73, -42, 55,
    -25, -61, 72, -1, 30, -58, -25, 63, 26, -48, -40, 26, -30, 60, 8, -17, -1, -18, -20, 43,
    -20, -4, -28, 127, -106, 29, 70, 64, -27, 39, -33, -5, -88, -40, -52, 26, 44, -17, 23, 2,
    -49, 22, -9, -8, 86, 49, -43, -60, 1, 10, 45, 36, -53, -4, 33, 38, 48, -72, 1, 19, 21, -65,
    4, -5, -62, 27, -25, 17, -6, 6, -45, -39, -46, 4, 26, 127, -9, 18, -33, -18, -3, 33, 2, -5,
    15, -26, -22, -117, -63, -17, -59, 61, -74, 7, -47, -58, -128, -67, 15, -16, -128, 12, 2,
    20, 9, -48, -40, 43, 3, -40, -16, -38, -6, -22, -28, -16, -59, -22, 6, -5, 11, -12, -66,
    -40, 27, -62, -44, -19, 38, -3, 39, -8, 40, -24, 13, 21, 50, -60, -22, 53, -29, -6, 1, 22,
    -59, 0, 17, -39, 115,
];
/// `layer1_bias` (src/mlp_data.c)
#[rustfmt::skip]
static layer1_bias: [i8; 72] = [
    -42, 20, 16, 0, 105, 60, 1, -97, 24, 60, 18, 13, 62, 25, 127, 34, 79, 55, 118, 127, 95, 31,
    -4, 87, 21, 12, 2, -14, 18, 23, 8, 17, -1, -8, 5, 4, 24, 37, 21, 13, 36, 13, 17, 18, 37, 30,
    33, 1, 8, -16, -11, -5, -31, -3, -5, 0, 6, 3, 58, -7, -1, -16, 5, -13, 16, 10, -2, -14, 11,
    -4, 3, -11,
];
/// `layer2_weights` (src/mlp_data.c)
#[rustfmt::skip]
static layer2_weights: [i8; 48] = [
    -113, -88, 31, -128, -126, -61, 85, -35, 118, -128, -61, 127, -128, -17, -128, 127, 104, -9,
    -128, 33, 45, 127, 5, 83, 84, -128, -85, -128, -45, 48, -53, -128, 46, 127, -17, 125, 117,
    -41, -117, -91, -127, -68, -1, -89, -80, 32, 106, 7,
];
/// `layer2_bias` (src/mlp_data.c)
#[rustfmt::skip]
static layer2_bias: [i8; 2] = [
    14, 117,
];
/// `layer0` (`src/mlp_data.c`)
static layer0: AnalysisDenseLayer = AnalysisDenseLayer {
    bias: &layer0_bias,
    input_weights: &layer0_weights,
    nb_inputs: 25,
    nb_neurons: 32,
    sigmoid: 0,
};
/// `layer1` (`src/mlp_data.c`)
static layer1: AnalysisGRULayer = AnalysisGRULayer {
    bias: &layer1_bias,
    input_weights: &layer1_weights,
    recurrent_weights: &layer1_recur_weights,
    nb_inputs: 32,
    nb_neurons: 24,
};
/// `layer2` (`src/mlp_data.c`)
static layer2: AnalysisDenseLayer = AnalysisDenseLayer {
    bias: &layer2_bias,
    input_weights: &layer2_weights,
    nb_inputs: 24,
    nb_neurons: 2,
    sigmoid: 1,
};

    /* --------------------------------------------------------------------- *
     * `src/mlp.h` / `src/mlp.c` / `src/mlp_data.c`
     * --------------------------------------------------------------------- */

    /// `WEIGHTS_SCALE (1.f/128)`
    const WEIGHTS_SCALE: f32 = 1.0 / 128.0;
    /// `MAX_NEURONS`
    pub const MAX_NEURONS: i32 = 32;

    /// `AnalysisDenseLayer` (`src/mlp.h`).
    pub struct AnalysisDenseLayer {
        pub bias: &'static [i8],
        pub input_weights: &'static [i8],
        pub nb_inputs: i32,
        pub nb_neurons: i32,
        pub sigmoid: i32,
    }

    /// `AnalysisGRULayer` (`src/mlp.h`).
    pub struct AnalysisGRULayer {
        pub bias: &'static [i8],
        pub input_weights: &'static [i8],
        pub recurrent_weights: &'static [i8],
        pub nb_inputs: i32,
        pub nb_neurons: i32,
    }

    /// `fmadd(a,b,c)` (`src/mlp.c`).
    #[inline(always)]
    fn fmadd(a: f32, b: f32, c: f32) -> f32 {
        a * b + c
    }

    /// `tansig_approx()` (`src/mlp.c:42`).
    fn tansig_approx(x: f32) -> f32 {
        const N0: f32 = 952.52801514;
        const N1: f32 = 96.39235687;
        const N2: f32 = 0.60863042;
        const D0: f32 = 952.72399902;
        const D1: f32 = 413.36801147;
        const D2: f32 = 11.88600922;
        let x2 = x * x;
        let mut num = fmadd(fmadd(N2, x2, N1), x2, N0);
        let den = fmadd(fmadd(D2, x2, D1), x2, D0);
        num = num * x / den;
        MAX32(-1.0f32, MIN32(1.0f32, num))
    }

    /// `sigmoid_approx()` (`src/mlp.c:57`).
    #[inline(always)]
    fn sigmoid_approx(x: f32) -> f32 {
        0.5f32 + 0.5f32 * tansig_approx(0.5f32 * x)
    }

    /// `gemm_accum()` (`src/mlp.c:62`).
    fn gemm_accum(out: &mut [f32], weights: &[i8], rows: i32, cols: i32, col_stride: i32, x: &[f32]) {
        for i in 0..rows {
            for j in 0..cols {
                out[i as usize] += (weights[(j * col_stride + i) as usize] as f32) * x[j as usize];
            }
        }
    }

    /// `analysis_compute_dense()` (`src/mlp.c:72`).
    fn analysis_compute_dense(layer: &AnalysisDenseLayer, output: &mut [f32], input: &[f32]) {
        let M = layer.nb_inputs;
        let N = layer.nb_neurons;
        let stride = N;
        for i in 0..N {
            output[i as usize] = layer.bias[i as usize] as f32;
        }
        gemm_accum(output, layer.input_weights, N, M, stride, input);
        for i in 0..N {
            output[i as usize] *= WEIGHTS_SCALE;
        }
        if layer.sigmoid != 0 {
            for i in 0..N {
                output[i as usize] = sigmoid_approx(output[i as usize]);
            }
        } else {
            for i in 0..N {
                output[i as usize] = tansig_approx(output[i as usize]);
            }
        }
    }

    /// `analysis_compute_gru()` (`src/mlp.c:94`).
    fn analysis_compute_gru(gru: &AnalysisGRULayer, state: &mut [f32], input: &[f32]) {
        let M = gru.nb_inputs;
        let N = gru.nb_neurons;
        let stride = 3 * N;
        let mut tmp = [0.0f32; MAX_NEURONS as usize];
        let mut z = [0.0f32; MAX_NEURONS as usize];
        let mut r = [0.0f32; MAX_NEURONS as usize];
        let mut h = [0.0f32; MAX_NEURONS as usize];
        /* Compute update gate. */
        for i in 0..N {
            z[i as usize] = gru.bias[i as usize] as f32;
        }
        gemm_accum(&mut z, gru.input_weights, N, M, stride, input);
        gemm_accum(&mut z, gru.recurrent_weights, N, N, stride, state);
        for i in 0..N {
            z[i as usize] = sigmoid_approx(WEIGHTS_SCALE * z[i as usize]);
        }

        /* Compute reset gate. */
        for i in 0..N {
            r[i as usize] = gru.bias[(N + i) as usize] as f32;
        }
        gemm_accum(&mut r, &gru.input_weights[N as usize..], N, M, stride, input);
        gemm_accum(&mut r, &gru.recurrent_weights[N as usize..], N, N, stride, state);
        for i in 0..N {
            r[i as usize] = sigmoid_approx(WEIGHTS_SCALE * r[i as usize]);
        }

        /* Compute output. */
        for i in 0..N {
            h[i as usize] = gru.bias[(2 * N + i) as usize] as f32;
        }
        for i in 0..N {
            tmp[i as usize] = state[i as usize] * r[i as usize];
        }
        gemm_accum(&mut h, &gru.input_weights[(2 * N) as usize..], N, M, stride, input);
        gemm_accum(
            &mut h,
            &gru.recurrent_weights[(2 * N) as usize..],
            N,
            N,
            stride,
            &tmp,
        );
        for i in 0..N {
            h[i as usize] = z[i as usize] * state[i as usize]
                + (1.0 - z[i as usize]) * tansig_approx(WEIGHTS_SCALE * h[i as usize]);
        }
        for i in 0..N {
            state[i as usize] = h[i as usize];
        }
    }

    /* --------------------------------------------------------------------- *
     * downmix
     * --------------------------------------------------------------------- */

    /// The C `const void *analysis_pcm` together with its `downmix_func`.  The
    /// enum variant *is* the downmix function, so `run_analysis` needs no
    /// separate function pointer.
    #[derive(Clone, Copy)]
    pub enum AnalysisPcm<'a> {
        /// `downmix_int`
        Int(&'a [i16]),
        /// `downmix_int24`
        Int24(&'a [i32]),
        /// `downmix_float`
        Float(&'a [f32]),
    }

    impl<'a> AnalysisPcm<'a> {
        #[inline(always)]
        fn downmix(self, y: &mut [f32], subframe: i32, offset: i32, c1: i32, c2: i32, C: i32) {
            match self {
                AnalysisPcm::Int(x) => downmix_int(x, y, subframe, offset, c1, c2, C),
                AnalysisPcm::Int24(x) => downmix_int24(x, y, subframe, offset, c1, c2, C),
                AnalysisPcm::Float(x) => downmix_float(x, y, subframe, offset, c1, c2, C),
            }
        }
    }

    /// `downmix_float()` (`src/opus_encoder.c:780`).
    fn downmix_float(x: &[f32], y: &mut [f32], subframe: i32, offset: i32, c1: i32, c2: i32, C: i32) {
        for j in 0..subframe {
            y[j as usize] = FLOAT2SIG(x[((j + offset) * C + c1) as usize]);
        }
        if c2 > -1 {
            for j in 0..subframe {
                y[j as usize] += FLOAT2SIG(x[((j + offset) * C + c2) as usize]);
            }
        } else if c2 == -2 {
            for c in 1..C {
                for j in 0..subframe {
                    y[j as usize] += FLOAT2SIG(x[((j + offset) * C + c) as usize]);
                }
            }
        }
        /* Cap signal to +6 dBFS to avoid problems in the analysis. */
        for j in 0..subframe {
            if y[j as usize] < -65536.0f32 {
                y[j as usize] = -65536.0f32;
            }
            if y[j as usize] > 65536.0f32 {
                y[j as usize] = 65536.0f32;
            }
            if celt_isnan(y[j as usize]) {
                y[j as usize] = 0.0;
            }
        }
    }

    /// `downmix_int()` (`src/opus_encoder.c:813`).
    fn downmix_int(x: &[i16], y: &mut [f32], subframe: i32, offset: i32, c1: i32, c2: i32, C: i32) {
        for j in 0..subframe {
            y[j as usize] = INT16TOSIG(x[((j + offset) * C + c1) as usize]);
        }
        if c2 > -1 {
            for j in 0..subframe {
                y[j as usize] += INT16TOSIG(x[((j + offset) * C + c2) as usize]);
            }
        } else if c2 == -2 {
            for c in 1..C {
                for j in 0..subframe {
                    y[j as usize] += INT16TOSIG(x[((j + offset) * C + c) as usize]);
                }
            }
        }
    }

    /// `downmix_int24()` (`src/opus_encoder.c:834`).
    fn downmix_int24(x: &[i32], y: &mut [f32], subframe: i32, offset: i32, c1: i32, c2: i32, C: i32) {
        for j in 0..subframe {
            y[j as usize] = INT24TOSIG(x[((j + offset) * C + c1) as usize]);
        }
        if c2 > -1 {
            for j in 0..subframe {
                y[j as usize] += INT24TOSIG(x[((j + offset) * C + c2) as usize]);
            }
        } else if c2 == -2 {
            for c in 1..C {
                for j in 0..subframe {
                    y[j as usize] += INT24TOSIG(x[((j + offset) * C + c) as usize]);
                }
            }
        }
    }

    /// `silk_resampler_down2_hp()` (`src/analysis.c:107`).
    fn silk_resampler_down2_hp(S: &mut [f32; 3], out: &mut [f32], in_: &[f32], inLen: i32) -> f32 {
        let len2 = inLen / 2;
        let mut hp_ener: f32 = 0.0;
        /* Internal variables and state are in Q10 format */
        for k in 0..len2 {
            /* Convert to Q10 */
            let mut in32 = in_[(2 * k) as usize];

            /* All-pass section for even input sample */
            let mut Y = SUB32(in32, S[0]);
            let mut X = MULT16_32_Q15(QCONST16(0.6074371f32, 15), Y);
            let mut out32 = ADD32(S[0], X);
            S[0] = ADD32(in32, X);
            let mut out32_hp = out32;
            /* Convert to Q10 */
            in32 = in_[(2 * k + 1) as usize];

            /* All-pass section for odd input sample, and add to output of previous section */
            Y = SUB32(in32, S[1]);
            X = MULT16_32_Q15(QCONST16(0.15063f32, 15), Y);
            out32 = ADD32(out32, S[1]);
            out32 = ADD32(out32, X);
            S[1] = ADD32(in32, X);

            Y = SUB32(-in32, S[2]);
            X = MULT16_32_Q15(QCONST16(0.15063f32, 15), Y);
            out32_hp = ADD32(out32_hp, S[2]);
            out32_hp = ADD32(out32_hp, X);
            S[2] = ADD32(-in32, X);

            /* len2 can be up to 480, so we shift by 8 to make it fit. */
            hp_ener += SHR64(out32_hp * out32_hp, 8);
            /* Add, convert back to int16 and store to output */
            out[k as usize] = HALF32(out32);
        }
        hp_ener
    }

    /// `downmix_and_resample()` (`src/analysis.c:176`).
    fn downmix_and_resample(
        x: AnalysisPcm,
        y: &mut [f32],
        S: &mut [f32; 3],
        subframe: i32,
        offset: i32,
        c1: i32,
        c2: i32,
        C: i32,
        Fs: i32,
    ) -> f32 {
        let mut ret: f32 = 0.0;

        if subframe == 0 {
            return 0.0;
        }
        let mut subframe = subframe;
        let mut offset = offset;
        if Fs == 48000 {
            subframe *= 2;
            offset *= 2;
        } else if Fs == 16000 {
            subframe = subframe * 2 / 3;
            offset = offset * 2 / 3;
        } else if Fs != 24000 {
            celt_assert!(false);
        }
        let mut tmp = vec![0.0f32; subframe as usize];

        x.downmix(&mut tmp, subframe, offset, c1, c2, C);
        if (c2 == -2 && C == 2) || c2 > -1 {
            for j in 0..subframe as usize {
                tmp[j] = HALF32(tmp[j]);
            }
        }
        if Fs == 48000 {
            ret = silk_resampler_down2_hp(S, y, &tmp, subframe);
        } else if Fs == 24000 {
            y[..subframe as usize].copy_from_slice(&tmp);
        } else if Fs == 16000 {
            let mut tmp3x = vec![0.0f32; (3 * subframe) as usize];
            /* Don't do this at home! This resampler is horrible and it's only (barely)
               usable for the purpose of the analysis because we don't care about all
               the aliasing between 8 kHz and 12 kHz. */
            for j in 0..subframe as usize {
                tmp3x[3 * j] = tmp[j];
                tmp3x[3 * j + 1] = tmp[j];
                tmp3x[3 * j + 2] = tmp[j];
            }
            silk_resampler_down2_hp(S, y, &tmp3x, 3 * subframe);
        }
        ret *= 1.0f32 / 32768.0 / 32768.0;
        ret
    }

    /* --------------------------------------------------------------------- *
     * TonalityAnalysisState
     * --------------------------------------------------------------------- */

    /// `TonalityAnalysisState` (`src/analysis.h`).
    pub struct TonalityAnalysisState {
        pub arch: i32,
        pub application: i32,
        pub Fs: opus_int32,
        /* `TONALITY_ANALYSIS_RESET_START angle` */
        pub angle: [f32; 240],
        pub d_angle: [f32; 240],
        pub d2_angle: [f32; 240],
        pub inmem: [opus_val32; ANALYSIS_BUF_SIZE as usize],
        pub mem_fill: i32, /* number of usable samples in the buffer */
        pub prev_band_tonality: [f32; NB_TBANDS as usize],
        pub prev_tonality: f32,
        pub prev_bandwidth: i32,
        pub E: [[f32; NB_TBANDS as usize]; NB_FRAMES as usize],
        pub logE: [[f32; NB_TBANDS as usize]; NB_FRAMES as usize],
        pub lowE: [f32; NB_TBANDS as usize],
        pub highE: [f32; NB_TBANDS as usize],
        pub meanE: [f32; (NB_TBANDS + 1) as usize],
        pub mem: [f32; 32],
        pub cmean: [f32; 8],
        pub std: [f32; 9],
        pub Etracker: f32,
        pub lowECount: f32,
        pub E_count: i32,
        pub count: i32,
        pub analysis_offset: i32,
        pub write_pos: i32,
        pub read_pos: i32,
        pub read_subframe: i32,
        pub hp_ener_accum: f32,
        pub initialized: i32,
        pub rnn_state: [f32; MAX_NEURONS as usize],
        pub downmix_state: [opus_val32; 3],
        pub info: [AnalysisInfo; DETECT_SIZE as usize],
    }

    impl Default for TonalityAnalysisState {
        fn default() -> Self {
            TonalityAnalysisState {
                arch: 0,
                application: 0,
                Fs: 0,
                angle: [0.0; 240],
                d_angle: [0.0; 240],
                d2_angle: [0.0; 240],
                inmem: [0.0; ANALYSIS_BUF_SIZE as usize],
                mem_fill: 0,
                prev_band_tonality: [0.0; NB_TBANDS as usize],
                prev_tonality: 0.0,
                prev_bandwidth: 0,
                E: [[0.0; NB_TBANDS as usize]; NB_FRAMES as usize],
                logE: [[0.0; NB_TBANDS as usize]; NB_FRAMES as usize],
                lowE: [0.0; NB_TBANDS as usize],
                highE: [0.0; NB_TBANDS as usize],
                meanE: [0.0; (NB_TBANDS + 1) as usize],
                mem: [0.0; 32],
                cmean: [0.0; 8],
                std: [0.0; 9],
                Etracker: 0.0,
                lowECount: 0.0,
                E_count: 0,
                count: 0,
                analysis_offset: 0,
                write_pos: 0,
                read_pos: 0,
                read_subframe: 0,
                hp_ener_accum: 0.0,
                initialized: 0,
                rnn_state: [0.0; MAX_NEURONS as usize],
                downmix_state: [0.0; 3],
                info: [AnalysisInfo::default(); DETECT_SIZE as usize],
            }
        }
    }

    impl TonalityAnalysisState {
        /// `tonality_analysis_reset()` (`src/analysis.c:223`): `OPUS_CLEAR` from
        /// `angle` to the end of the struct (`arch`, `application` and `Fs` are
        /// preserved).
        pub fn tonality_analysis_reset(&mut self) {
            self.angle = [0.0; 240];
            self.d_angle = [0.0; 240];
            self.d2_angle = [0.0; 240];
            self.inmem = [0.0; ANALYSIS_BUF_SIZE as usize];
            self.mem_fill = 0;
            self.prev_band_tonality = [0.0; NB_TBANDS as usize];
            self.prev_tonality = 0.0;
            self.prev_bandwidth = 0;
            self.E = [[0.0; NB_TBANDS as usize]; NB_FRAMES as usize];
            self.logE = [[0.0; NB_TBANDS as usize]; NB_FRAMES as usize];
            self.lowE = [0.0; NB_TBANDS as usize];
            self.highE = [0.0; NB_TBANDS as usize];
            self.meanE = [0.0; (NB_TBANDS + 1) as usize];
            self.mem = [0.0; 32];
            self.cmean = [0.0; 8];
            self.std = [0.0; 9];
            self.Etracker = 0.0;
            self.lowECount = 0.0;
            self.E_count = 0;
            self.count = 0;
            self.analysis_offset = 0;
            self.write_pos = 0;
            self.read_pos = 0;
            self.read_subframe = 0;
            self.hp_ener_accum = 0.0;
            self.initialized = 0;
            self.rnn_state = [0.0; MAX_NEURONS as usize];
            self.downmix_state = [0.0; 3];
            self.info = [AnalysisInfo::default(); DETECT_SIZE as usize];
        }
    }

    /// `tonality_analysis_init()` (`src/analysis.c:216`).
    pub fn tonality_analysis_init(tonal: &mut TonalityAnalysisState, Fs: opus_int32) {
        /* Initialize reusable fields. */
        tonal.arch = opus_select_arch();
        tonal.Fs = Fs;
        /* Clear remaining fields. */
        tonal.tonality_analysis_reset();
    }

    /// `tonality_get_info()` (`src/analysis.c:228`).
    pub fn tonality_get_info(
        tonal: &mut TonalityAnalysisState,
        info_out: &mut AnalysisInfo,
        len: i32,
    ) {
        let mut pos;
        let curr_lookahead;
        let mut tonality_max;
        let mut tonality_avg;
        let mut tonality_count;
        let pos0;
        let mut prob_avg;
        let mut prob_count;
        let mut prob_min;
        let mut prob_max;
        let vad_prob;
        let mut mpos;
        let mut vpos;
        let mut bandwidth_span;

        pos = tonal.read_pos;
        curr_lookahead = tonal.write_pos - tonal.read_pos;
        let curr_lookahead = if curr_lookahead < 0 {
            curr_lookahead + DETECT_SIZE
        } else {
            curr_lookahead
        };

        tonal.read_subframe += len / (tonal.Fs / 400);
        while tonal.read_subframe >= 8 {
            tonal.read_subframe -= 8;
            tonal.read_pos += 1;
        }
        if tonal.read_pos >= DETECT_SIZE {
            tonal.read_pos -= DETECT_SIZE;
        }

        /* On long frames, look at the second analysis window rather than the first. */
        if len > tonal.Fs / 50 && pos != tonal.write_pos {
            pos += 1;
            if pos == DETECT_SIZE {
                pos = 0;
            }
        }
        if pos == tonal.write_pos {
            pos -= 1;
        }
        if pos < 0 {
            pos = DETECT_SIZE - 1;
        }
        pos0 = pos;
        *info_out = tonal.info[pos as usize];
        if info_out.valid == 0 {
            return;
        }
        tonality_max = info_out.tonality;
        tonality_avg = info_out.tonality;
        tonality_count = 1;
        /* Look at the neighbouring frames and pick largest bandwidth found (to be safe). */
        bandwidth_span = 6;
        /* If possible, look ahead for a tone to compensate for the delay in the tone detector. */
        for _i in 0..3 {
            pos += 1;
            if pos == DETECT_SIZE {
                pos = 0;
            }
            if pos == tonal.write_pos {
                break;
            }
            tonality_max = MAX32(tonality_max, tonal.info[pos as usize].tonality);
            tonality_avg += tonal.info[pos as usize].tonality;
            tonality_count += 1;
            info_out.bandwidth = IMAX(info_out.bandwidth, tonal.info[pos as usize].bandwidth);
            bandwidth_span -= 1;
        }
        pos = pos0;
        /* Look back in time to see if any has a wider bandwidth than the current frame. */
        for _i in 0..bandwidth_span {
            pos -= 1;
            if pos < 0 {
                pos = DETECT_SIZE - 1;
            }
            if pos == tonal.write_pos {
                break;
            }
            info_out.bandwidth = IMAX(info_out.bandwidth, tonal.info[pos as usize].bandwidth);
        }
        info_out.tonality = MAX32(tonality_avg / (tonality_count as f32), tonality_max - 0.2f32);

        mpos = pos0;
        vpos = pos0;
        /* If we have enough look-ahead, compensate for the ~5-frame delay in the music prob and
           ~1 frame delay in the VAD prob. */
        if curr_lookahead > 15 {
            mpos += 5;
            if mpos >= DETECT_SIZE {
                mpos -= DETECT_SIZE;
            }
            vpos += 1;
            if vpos >= DETECT_SIZE {
                vpos -= DETECT_SIZE;
            }
        }

        prob_min = 1.0f32;
        prob_max = 0.0f32;
        vad_prob = tonal.info[vpos as usize].activity_probability;
        prob_count = MAX16(0.1f32, vad_prob);
        prob_avg = MAX16(0.1f32, vad_prob) * tonal.info[mpos as usize].music_prob;
        loop {
            let pos_vad;
            mpos += 1;
            if mpos == DETECT_SIZE {
                mpos = 0;
            }
            if mpos == tonal.write_pos {
                break;
            }
            vpos += 1;
            if vpos == DETECT_SIZE {
                vpos = 0;
            }
            if vpos == tonal.write_pos {
                break;
            }
            pos_vad = tonal.info[vpos as usize].activity_probability;
            prob_min = MIN16(
                (prob_avg - TRANSITION_PENALTY * (vad_prob - pos_vad)) / prob_count,
                prob_min,
            );
            prob_max = MAX16(
                (prob_avg + TRANSITION_PENALTY * (vad_prob - pos_vad)) / prob_count,
                prob_max,
            );
            prob_count += MAX16(0.1f32, pos_vad);
            prob_avg += MAX16(0.1f32, pos_vad) * tonal.info[mpos as usize].music_prob;
        }
        info_out.music_prob = prob_avg / prob_count;
        prob_min = MIN16(prob_avg / prob_count, prob_min);
        prob_max = MAX16(prob_avg / prob_count, prob_max);
        prob_min = MAX16(prob_min, 0.0f32);
        prob_max = MIN16(prob_max, 1.0f32);

        /* If we don't have enough look-ahead, do our best to make a decent decision. */
        if curr_lookahead < 10 {
            let mut pmin;
            let mut pmax;
            pmin = prob_min;
            pmax = prob_max;
            pos = pos0;
            /* Look for min/max in the past. */
            for _i in 0..IMIN(tonal.count - 1, 15) {
                pos -= 1;
                if pos < 0 {
                    pos = DETECT_SIZE - 1;
                }
                pmin = MIN16(pmin, tonal.info[pos as usize].music_prob);
                pmax = MAX16(pmax, tonal.info[pos as usize].music_prob);
            }
            /* Bias against switching on active audio. */
            pmin = MAX16(0.0f32, pmin - 0.1f32 * vad_prob);
            pmax = MIN16(1.0f32, pmax + 0.1f32 * vad_prob);
            prob_min += (1.0f32 - 0.1f32 * (curr_lookahead as f32)) * (pmin - prob_min);
            prob_max += (1.0f32 - 0.1f32 * (curr_lookahead as f32)) * (pmax - prob_max);
        }
        info_out.music_prob_min = prob_min;
        info_out.music_prob_max = prob_max;
    }

    /// `tonality_analysis()` (`src/analysis.c:430`).
    fn tonality_analysis(
        tonal: &mut TonalityAnalysisState,
        celt_mode: &CELTMode,
        x: AnalysisPcm,
        len: i32,
        offset: i32,
        c1: i32,
        c2: i32,
        C: i32,
        lsb_depth: i32,
    ) {
        let N = 480;
        let N2 = 240;
        /* `VARDECL(kiss_fft_cpx, in_/out); ALLOC(..., 480, ...)` and
           `VARDECL(float, tonality/noisiness); ALLOC(..., 240, ...)` — the C
           sizes are compile-time constants here, so these are stack arrays. */
        let mut in_ = [kiss_fft_cpx::default(); 480];
        let mut out = [kiss_fft_cpx::default(); 480];
        let mut tonality = [0.0f32; 240];
        let mut noisiness = [0.0f32; 240];
        let mut band_tonality = [0.0f32; NB_TBANDS as usize];
        let mut logE = [0.0f32; NB_TBANDS as usize];
        let mut BFCC = [0.0f32; 8];
        let mut features = [0.0f32; 25];
        let mut frame_tonality: f32;
        let mut max_frame_tonality: f32;
        let mut frame_noisiness: f32;
        let pi4 = (M_PI * M_PI * M_PI * M_PI) as f32;
        let mut slope: f32 = 0.0;
        let mut frame_stationarity: f32;
        let mut relativeE: f32;
        let mut frame_probs = [0.0f32; 2];
        let mut alpha;
        let mut alphaE;
        let mut alphaE2;
        let mut frame_loudness: f32;
        let mut bandwidth_mask: f32;
        let mut is_masked = [0i32; (NB_TBANDS + 1) as usize];
        let mut bandwidth: i32 = 0;
        let mut maxE: f32 = 0.0;
        let mut noise_floor;
        let mut remaining;
        let mut hp_ener;
        let mut tonality2 = [0.0f32; 240];
        let mut midE = [0.0f32; 8];
        let mut spec_variability: f32 = 0.0;
        let mut band_log2 = [0.0f32; (NB_TBANDS + 1) as usize];
        let mut leakage_from = [0.0f32; (NB_TBANDS + 1) as usize];
        let mut leakage_to = [0.0f32; (NB_TBANDS + 1) as usize];
        let mut layer_out = [0.0f32; MAX_NEURONS as usize];
        let mut below_max_pitch: f32;
        let mut above_max_pitch: f32;
        let is_silence;

        if tonal.initialized == 0 {
            tonal.mem_fill = 240;
            tonal.initialized = 1;
        }
        alpha = 1.0f32 / (IMIN(10, 1 + tonal.count) as f32);
        alphaE = 1.0f32 / (IMIN(25, 1 + tonal.count) as f32);
        /* Noise floor related decay for bandwidth detection: -2.2 dB/second */
        alphaE2 = 1.0f32 / (IMIN(100, 1 + tonal.count) as f32);
        if tonal.count <= 1 {
            alphaE2 = 1.0;
        }

        let mut len = len;
        let mut offset = offset;
        if tonal.Fs == 48000 {
            /* len and offset are now at 24 kHz. */
            len /= 2;
            offset /= 2;
        } else if tonal.Fs == 16000 {
            len = 3 * len / 2;
            offset = 3 * offset / 2;
        }

        let mf = tonal.mem_fill as usize;
        let n1 = IMIN(len, ANALYSIS_BUF_SIZE - tonal.mem_fill);
        let v = downmix_and_resample(
            x,
            &mut tonal.inmem[mf..],
            &mut tonal.downmix_state,
            n1,
            offset,
            c1,
            c2,
            C,
            tonal.Fs,
        );
        tonal.hp_ener_accum += v;
        if tonal.mem_fill + len < ANALYSIS_BUF_SIZE {
            tonal.mem_fill += len;
            /* Don't have enough to update the analysis */
            return;
        }
        hp_ener = tonal.hp_ener_accum;
        let info_idx = tonal.write_pos as usize;
        tonal.write_pos += 1;
        if tonal.write_pos >= DETECT_SIZE {
            tonal.write_pos -= DETECT_SIZE;
        }

        is_silence = super::is_digital_silence(&tonal.inmem, ANALYSIS_BUF_SIZE, 1, lsb_depth);

        {
            let kfft = celt_mode.mdct.kfft[0].as_ref().unwrap();
            let _ = kfft;
        }
        for i in 0..N2 {
            let w = analysis_window[i as usize];
            in_[i as usize].r = w * tonal.inmem[i as usize];
            in_[i as usize].i = w * tonal.inmem[(N2 + i) as usize];
            in_[(N - i - 1) as usize].r = w * tonal.inmem[(N - i - 1) as usize];
            in_[(N - i - 1) as usize].i = w * tonal.inmem[(N + N2 - i - 1) as usize];
        }
        tonal
            .inmem
            .copy_within((ANALYSIS_BUF_SIZE - 240) as usize..(ANALYSIS_BUF_SIZE - 240) as usize + 240, 0);
        remaining = len - (ANALYSIS_BUF_SIZE - tonal.mem_fill);
        let v = downmix_and_resample(
            x,
            &mut tonal.inmem[240..],
            &mut tonal.downmix_state,
            remaining,
            offset + ANALYSIS_BUF_SIZE - tonal.mem_fill,
            c1,
            c2,
            C,
            tonal.Fs,
        );
        tonal.hp_ener_accum = v;
        tonal.mem_fill = 240 + remaining;
        if is_silence != 0 {
            /* On silence, copy the previous analysis. */
            let mut prev_pos = tonal.write_pos - 2;
            if prev_pos < 0 {
                prev_pos += DETECT_SIZE;
            }
            tonal.info[info_idx] = tonal.info[prev_pos as usize];
            return;
        }
        let kfft = celt_mode.mdct.kfft[0].as_ref().unwrap();
        opus_fft(kfft, &in_, &mut out, tonal.arch);

        /* If there's any NaN on the input, the entire output will be NaN, so we only need to check one value. */
        if celt_isnan(out[0].r) {
            tonal.info[info_idx].valid = 0;
            return;
        }

        for i in 1..N2 {
            let X1r;
            let X1i;
            let X2r;
            let X2i;
            let angle;
            let d_angle;
            let mut d2_angle;
            let angle2;
            let d_angle2;
            let mut d2_angle2;
            let mut mod1;
            let mut mod2;
            let avg_mod;
            X1r = out[i as usize].r + out[(N - i) as usize].r;
            X1i = out[i as usize].i - out[(N - i) as usize].i;
            X2r = out[i as usize].i + out[(N - i) as usize].i;
            X2i = out[(N - i) as usize].r - out[i as usize].r;

            angle = ((0.5f64 / M_PI) as f32) * fast_atan2f(X1i, X1r);
            d_angle = angle - tonal.angle[i as usize];
            d2_angle = d_angle - tonal.d_angle[i as usize];

            angle2 = ((0.5f64 / M_PI) as f32) * fast_atan2f(X2i, X2r);
            d_angle2 = angle2 - angle;
            d2_angle2 = d_angle2 - d_angle;

            mod1 = d2_angle - (float2int(d2_angle) as f32);
            noisiness[i as usize] = ABS16(mod1);
            mod1 *= mod1;
            mod1 *= mod1;

            mod2 = d2_angle2 - (float2int(d2_angle2) as f32);
            noisiness[i as usize] += ABS16(mod2);
            mod2 *= mod2;
            mod2 *= mod2;

            avg_mod = 0.25f32 * (tonal.d2_angle[i as usize] + mod1 + 2.0 * mod2);
            /* This introduces an extra delay of 2 frames in the detection. */
            tonality[i as usize] = 1.0f32 / (1.0f32 + 40.0f32 * 16.0f32 * pi4 * avg_mod) - 0.015f32;
            /* No delay on this detection, but it's less reliable. */
            tonality2[i as usize] = 1.0f32 / (1.0f32 + 40.0f32 * 16.0f32 * pi4 * mod2) - 0.015f32;

            tonal.angle[i as usize] = angle2;
            tonal.d_angle[i as usize] = d_angle2;
            tonal.d2_angle[i as usize] = mod2;
        }
        for i in 2..N2 - 1 {
            let tt = MIN32(
                tonality2[i as usize],
                MAX32(tonality2[(i - 1) as usize], tonality2[(i + 1) as usize]),
            );
            tonality[i as usize] = 0.9f32 * MAX32(tonality[i as usize], tt - 0.1f32);
        }
        frame_tonality = 0.0;
        max_frame_tonality = 0.0;
        tonal.info[info_idx].activity = 0.0;
        frame_noisiness = 0.0;
        frame_stationarity = 0.0;
        if tonal.count == 0 {
            for b in 0..NB_TBANDS {
                tonal.lowE[b as usize] = 1e10;
                tonal.highE[b as usize] = -1e10;
            }
        }
        relativeE = 0.0;
        frame_loudness = 0.0;
        /* The energy of the very first band is special because of DC. */
        {
            let mut E: f32 = 0.0;
            let X1r;
            let X2r;
            X1r = 2.0 * out[0].r;
            X2r = 2.0 * out[0].i;
            E = X1r * X1r + X2r * X2r;
            for i in 1..4 {
                let binE = out[i as usize].r * out[i as usize].r
                    + out[(N - i) as usize].r * out[(N - i) as usize].r
                    + out[i as usize].i * out[i as usize].i
                    + out[(N - i) as usize].i * out[(N - i) as usize].i;
                E += binE;
            }
            E = SCALE_ENER(E);
            band_log2[0] = 0.5f32 * 1.442695f32 * (((E + 1e-10f32) as f64).ln() as f32);
        }
        let ecount = tonal.E_count as usize;
        for b in 0..NB_TBANDS {
            let mut E: f32 = 0.0;
            let mut tE: f32 = 0.0;
            let mut nE: f32 = 0.0;
            let mut stationarity;
            for i in tbands[b as usize]..tbands[(b + 1) as usize] {
                let mut binE = out[i as usize].r * out[i as usize].r
                    + out[(N - i) as usize].r * out[(N - i) as usize].r
                    + out[i as usize].i * out[i as usize].i
                    + out[(N - i) as usize].i * out[(N - i) as usize].i;
                binE = SCALE_ENER(binE);
                E += binE;
                tE += binE * MAX32(0.0, tonality[i as usize]);
                nE += binE * 2.0f32 * (0.5f32 - noisiness[i as usize]);
            }
            /* Check for extreme band energies that could cause NaNs later. */
            if !(E < 1e9f32) || celt_isnan(E) {
                tonal.info[info_idx].valid = 0;
                return;
            }

            tonal.E[ecount][b as usize] = E;
            frame_noisiness += nE / (1e-15f32 + E);

            frame_loudness += ((E + 1e-10f32) as f64).sqrt() as f32;
            logE[b as usize] = ((E + 1e-10f32) as f64).ln() as f32;
            band_log2[(b + 1) as usize] =
                0.5f32 * 1.442695f32 * (((E + 1e-10f32) as f64).ln() as f32);
            tonal.logE[ecount][b as usize] = logE[b as usize];
            if tonal.count == 0 {
                tonal.highE[b as usize] = logE[b as usize];
                tonal.lowE[b as usize] = logE[b as usize];
            }
            if tonal.highE[b as usize] > tonal.lowE[b as usize] + 7.5 {
                if tonal.highE[b as usize] - logE[b as usize]
                    > logE[b as usize] - tonal.lowE[b as usize]
                {
                    tonal.highE[b as usize] -= 0.01f32;
                } else {
                    tonal.lowE[b as usize] += 0.01f32;
                }
            }
            if logE[b as usize] > tonal.highE[b as usize] {
                tonal.highE[b as usize] = logE[b as usize];
                tonal.lowE[b as usize] = MAX32(tonal.highE[b as usize] - 15.0, tonal.lowE[b as usize]);
            } else if logE[b as usize] < tonal.lowE[b as usize] {
                tonal.lowE[b as usize] = logE[b as usize];
                tonal.highE[b as usize] = MIN32(tonal.lowE[b as usize] + 15.0, tonal.highE[b as usize]);
            }
            relativeE += (logE[b as usize] - tonal.lowE[b as usize])
                / (1e-5f32 + (tonal.highE[b as usize] - tonal.lowE[b as usize]));

            let mut L1: f32 = 0.0;
            let mut L2: f32 = 0.0;
            for i in 0..NB_FRAMES {
                L1 += (tonal.E[i as usize][b as usize] as f64).sqrt() as f32;
                L2 += tonal.E[i as usize][b as usize];
            }

            stationarity = MIN16(
                0.99f32,
                L1 / (((1e-15f64 + ((NB_FRAMES as f32) * L2) as f64)).sqrt() as f32),
            );
            stationarity *= stationarity;
            stationarity *= stationarity;
            frame_stationarity += stationarity;
            band_tonality[b as usize] = MAX16(
                tE / (1e-15f32 + E),
                stationarity * tonal.prev_band_tonality[b as usize],
            );
            frame_tonality += band_tonality[b as usize];
            if b >= NB_TBANDS - NB_TONAL_SKIP_BANDS {
                frame_tonality -= band_tonality[(b - NB_TBANDS + NB_TONAL_SKIP_BANDS) as usize];
            }
            max_frame_tonality = MAX16(
                max_frame_tonality,
                (1.0f32 + 0.03f32 * ((b - NB_TBANDS) as f32)) * frame_tonality,
            );
            slope += band_tonality[b as usize] * ((b - 8) as f32);
            tonal.prev_band_tonality[b as usize] = band_tonality[b as usize];
        }

        leakage_from[0] = band_log2[0];
        leakage_to[0] = band_log2[0] - LEAKAGE_OFFSET;
        for b in 1..NB_TBANDS + 1 {
            let leak_slope =
                LEAKAGE_SLOPE * ((tbands[b as usize] - tbands[(b - 1) as usize]) as f32) / 4.0;
            leakage_from[b as usize] = MIN16(leakage_from[(b - 1) as usize] + leak_slope, band_log2[b as usize]);
            leakage_to[b as usize] = MAX16(
                leakage_to[(b - 1) as usize] - leak_slope,
                band_log2[b as usize] - LEAKAGE_OFFSET,
            );
        }
        let mut b = NB_TBANDS - 2;
        while b >= 0 {
            let leak_slope =
                LEAKAGE_SLOPE * ((tbands[(b + 1) as usize] - tbands[b as usize]) as f32) / 4.0;
            leakage_from[b as usize] = MIN16(leakage_from[(b + 1) as usize] + leak_slope, leakage_from[b as usize]);
            leakage_to[b as usize] = MAX16(leakage_to[(b + 1) as usize] - leak_slope, leakage_to[b as usize]);
            b -= 1;
        }
        celt_assert!(NB_TBANDS + 1 <= LEAK_BANDS);
        for b in 0..NB_TBANDS + 1 {
            /* leak_boost[] is made up of two terms. The first, based on leakage_to[],
               represents the boost needed to overcome the amount of analysis leakage
               cause in a weaker band b by louder neighbouring bands.
               The second, based on leakage_from[], applies to a loud band b for
               which the quantization noise causes synthesis leakage to the weaker
               neighbouring bands. */
            let boost = MAX16(0.0f32, leakage_to[b as usize] - band_log2[b as usize])
                + MAX16(
                    0.0f32,
                    band_log2[b as usize] - (leakage_from[b as usize] + LEAKAGE_OFFSET),
                );
            tonal.info[info_idx].leak_boost[b as usize] =
                IMIN(255, (0.5f64 + (64.0f32 * boost) as f64).floor() as i32) as u8;
        }
        let mut bb = NB_TBANDS + 1;
        while bb < LEAK_BANDS {
            tonal.info[info_idx].leak_boost[bb as usize] = 0;
            bb += 1;
        }

        for i in 0..NB_FRAMES {
            let mut mindist: f32 = 1e15;
            for j in 0..NB_FRAMES {
                let mut dist: f32 = 0.0;
                for k in 0..NB_TBANDS {
                    let tmp = tonal.logE[i as usize][k as usize] - tonal.logE[j as usize][k as usize];
                    dist += tmp * tmp;
                }
                if j != i {
                    mindist = MIN32(mindist, dist);
                }
            }
            spec_variability += mindist;
        }
        spec_variability = (((spec_variability / (NB_FRAMES as f32)) / (NB_TBANDS as f32)) as f64).sqrt()
            as f32;
        bandwidth_mask = 0.0;
        bandwidth = 0;
        maxE = 0.0;
        noise_floor = 5.7e-4f32 / ((1i32 << IMAX(0, lsb_depth - 8)) as f32);
        noise_floor *= noise_floor;
        below_max_pitch = 0.0;
        above_max_pitch = 0.0;
        for b in 0..NB_TBANDS {
            let mut E: f32 = 0.0;
            let Em;
            let band_start;
            let band_end;
            /* Keep a margin of 300 Hz for aliasing */
            band_start = tbands[b as usize];
            band_end = tbands[(b + 1) as usize];
            for i in band_start..band_end {
                let binE = out[i as usize].r * out[i as usize].r
                    + out[(N - i) as usize].r * out[(N - i) as usize].r
                    + out[i as usize].i * out[i as usize].i
                    + out[(N - i) as usize].i * out[(N - i) as usize].i;
                E += binE;
            }
            E = SCALE_ENER(E);
            maxE = MAX32(maxE, E);
            if band_start < 64 {
                below_max_pitch += E;
            } else {
                above_max_pitch += E;
            }
            tonal.meanE[b as usize] = MAX32((1.0 - alphaE2) * tonal.meanE[b as usize], E);
            Em = MAX32(E, tonal.meanE[b as usize]);
            /* Consider the band "active" only if all these conditions are met:
               1) less than 90 dB below the peak band (maximal masking possible considering
                  both the ATH and the loudness-dependent slope of the spreading function)
               2) above the PCM quantization noise floor
               We use b+1 because the first CELT band isn't included in tbands[]
            */
            if E * 1e9f32 > maxE
                && (Em > 3.0 * noise_floor * ((band_end - band_start) as f32)
                    || E > noise_floor * ((band_end - band_start) as f32))
            {
                bandwidth = b + 1;
            }
            /* Check if the band is masked (see below). */
            is_masked[b as usize] = (E
                < (if tonal.prev_bandwidth >= b + 1 { 0.01f32 } else { 0.05f32 }) * bandwidth_mask)
                as i32;
            /* Use a simple follower with 13 dB/Bark slope for spreading function. */
            bandwidth_mask = MAX32(0.05f32 * bandwidth_mask, E);
        }
        /* Special case for the last two bands, for which we don't have spectrum but only
           the energy above 12 kHz. The difficulty here is that the high-pass we use
           leaks some LF energy, so we need to increase the threshold without accidentally cutting
           off the band. */
        if tonal.Fs == 48000 {
            let noise_ratio;
            let Em;
            let E = hp_ener * (1.0f32 / (60.0 * 60.0));
            noise_ratio = if tonal.prev_bandwidth == 20 { 10.0f32 } else { 30.0f32 };

            above_max_pitch += E;
            tonal.meanE[NB_TBANDS as usize] =
                MAX32((1.0 - alphaE2) * tonal.meanE[NB_TBANDS as usize], E);
            Em = MAX32(E, tonal.meanE[NB_TBANDS as usize]);
            if Em > 3.0 * noise_ratio * noise_floor * 160.0
                || E > noise_ratio * noise_floor * 160.0
            {
                bandwidth = 20;
            }
            /* Check if the band is masked (see below). */
            is_masked[NB_TBANDS as usize] = (E
                < (if tonal.prev_bandwidth == 20 { 0.01f32 } else { 0.05f32 }) * bandwidth_mask)
                as i32;
        }
        if above_max_pitch > below_max_pitch {
            tonal.info[info_idx].max_pitch_ratio = below_max_pitch / above_max_pitch;
        } else {
            tonal.info[info_idx].max_pitch_ratio = 1.0;
        }
        /* In some cases, resampling aliasing can create a small amount of energy in the first band
           being cut. So if the last band is masked, we don't include it.  */
        if bandwidth == 20 && is_masked[NB_TBANDS as usize] != 0 {
            bandwidth -= 2;
        } else if bandwidth > 0 && bandwidth <= NB_TBANDS && is_masked[(bandwidth - 1) as usize] != 0 {
            bandwidth -= 1;
        }
        if tonal.count <= 2 {
            bandwidth = 20;
        }
        frame_loudness = 20.0 * ((frame_loudness as f64).log10() as f32);
        tonal.Etracker = MAX32(tonal.Etracker - 0.003f32, frame_loudness);
        tonal.lowECount *= 1.0 - alphaE;
        if frame_loudness < tonal.Etracker - 30.0 {
            tonal.lowECount += alphaE;
        }

        for i in 0..8 {
            let mut sum: f32 = 0.0;
            for b in 0..16 {
                sum += dct_table[(i * 16 + b) as usize] * logE[b as usize];
            }
            BFCC[i as usize] = sum;
        }
        for i in 0..8 {
            let mut sum: f32 = 0.0;
            for b in 0..16 {
                sum += dct_table[(i * 16 + b) as usize]
                    * 0.5f32
                    * (tonal.highE[b as usize] + tonal.lowE[b as usize]);
            }
            midE[i as usize] = sum;
        }

        frame_stationarity /= NB_TBANDS as f32;
        relativeE /= NB_TBANDS as f32;
        if tonal.count < 10 {
            relativeE = 0.5;
        }
        frame_noisiness /= NB_TBANDS as f32;
        tonal.info[info_idx].activity = frame_noisiness + (1.0 - frame_noisiness) * relativeE;
        frame_tonality = max_frame_tonality / ((NB_TBANDS - NB_TONAL_SKIP_BANDS) as f32);
        frame_tonality = MAX16(frame_tonality, tonal.prev_tonality * 0.8f32);
        tonal.prev_tonality = frame_tonality;

        slope /= 8.0 * 8.0;
        tonal.info[info_idx].tonality_slope = slope;

        tonal.E_count = (tonal.E_count + 1) % NB_FRAMES;
        tonal.count = IMIN(tonal.count + 1, ANALYSIS_COUNT_MAX);
        tonal.info[info_idx].tonality = frame_tonality;

        for i in 0..4 {
            features[i as usize] = -0.12299f32 * (BFCC[i as usize] + tonal.mem[(i + 24) as usize])
                + 0.49195f32 * (tonal.mem[i as usize] + tonal.mem[(i + 16) as usize])
                + 0.69693f32 * tonal.mem[(i + 8) as usize]
                - 1.4349f32 * tonal.cmean[i as usize];
        }

        for i in 0..4 {
            tonal.cmean[i as usize] = (1.0 - alpha) * tonal.cmean[i as usize] + alpha * BFCC[i as usize];
        }

        for i in 0..4 {
            features[(4 + i) as usize] = 0.63246f32 * (BFCC[i as usize] - tonal.mem[(i + 24) as usize])
                + 0.31623f32 * (tonal.mem[i as usize] - tonal.mem[(i + 16) as usize]);
        }
        for i in 0..3 {
            features[(8 + i) as usize] = 0.53452f32 * (BFCC[i as usize] + tonal.mem[(i + 24) as usize])
                - 0.26726f32 * (tonal.mem[i as usize] + tonal.mem[(i + 16) as usize])
                - 0.53452f32 * tonal.mem[(i + 8) as usize];
        }

        if tonal.count > 5 {
            for i in 0..9 {
                tonal.std[i as usize] = (1.0 - alpha) * tonal.std[i as usize]
                    + alpha * features[i as usize] * features[i as usize];
            }
        }
        for i in 0..4 {
            features[i as usize] = BFCC[i as usize] - midE[i as usize];
        }

        for i in 0..8 {
            tonal.mem[(i + 24) as usize] = tonal.mem[(i + 16) as usize];
            tonal.mem[(i + 16) as usize] = tonal.mem[(i + 8) as usize];
            tonal.mem[(i + 8) as usize] = tonal.mem[i as usize];
            tonal.mem[i as usize] = BFCC[i as usize];
        }
        for i in 0..9 {
            features[(11 + i) as usize] =
                ((tonal.std[i as usize] as f64).sqrt() as f32) - std_feature_bias[i as usize];
        }
        features[18] = spec_variability - 0.78f32;
        features[20] = tonal.info[info_idx].tonality - 0.154723f32;
        features[21] = tonal.info[info_idx].activity - 0.724643f32;
        features[22] = frame_stationarity - 0.743717f32;
        features[23] = tonal.info[info_idx].tonality_slope + 0.069216f32;
        features[24] = tonal.lowECount - 0.067930f32;

        analysis_compute_dense(&layer0, &mut layer_out, &features);
        analysis_compute_gru(&layer1, &mut tonal.rnn_state, &layer_out);
        analysis_compute_dense(&layer2, &mut frame_probs, &tonal.rnn_state);

        /* Probability of speech or music vs noise */
        tonal.info[info_idx].activity_probability = frame_probs[1];
        tonal.info[info_idx].music_prob = frame_probs[0];

        tonal.info[info_idx].bandwidth = bandwidth;
        tonal.prev_bandwidth = bandwidth;
        tonal.info[info_idx].noisiness = frame_noisiness;
        tonal.info[info_idx].valid = 1;
    }

    /// `run_analysis()` (`src/analysis.c:952`).
    #[allow(clippy::too_many_arguments)]
    pub fn run_analysis(
        analysis: &mut TonalityAnalysisState,
        celt_mode: &CELTMode,
        analysis_pcm: Option<AnalysisPcm>,
        analysis_frame_size: i32,
        frame_size: i32,
        c1: i32,
        c2: i32,
        C: i32,
        Fs: opus_int32,
        lsb_depth: i32,
        analysis_info: &mut AnalysisInfo,
    ) {
        let mut analysis_frame_size = analysis_frame_size - (analysis_frame_size & 1);
        if let Some(x) = analysis_pcm {
            /* Avoid overflow/wrap-around of the analysis buffer */
            analysis_frame_size = IMIN((DETECT_SIZE - 5) * Fs / 50, analysis_frame_size);

            let mut pcm_len = analysis_frame_size - analysis.analysis_offset;
            let mut offset = analysis.analysis_offset;
            while pcm_len > 0 {
                tonality_analysis(
                    analysis,
                    celt_mode,
                    x,
                    IMIN(Fs / 50, pcm_len),
                    offset,
                    c1,
                    c2,
                    C,
                    lsb_depth,
                );
                offset += Fs / 50;
                pcm_len -= Fs / 50;
            }
            analysis.analysis_offset = analysis_frame_size;

            analysis.analysis_offset -= frame_size;
        }

        tonality_get_info(analysis, analysis_info, frame_size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_encoder(Fs: i32, channels: i32, application: i32) -> Box<OpusEncoder> {
        let mut err = 0;
        OpusEncoder::create(Fs, channels, application, &mut err).expect("create must succeed")
    }

    #[test]
    fn encoder_size_matches_reference() {
        /* Verified against reference/lib-ref/libopus.a. */
        assert_eq!(OpusEncoder::get_size(1), 109108);
        assert_eq!(OpusEncoder::get_size(2), 126124);
        assert_eq!(OpusEncoder::get_size(0), 0);
        assert_eq!(OpusEncoder::get_size(3), 0);
    }

    #[test]
    fn frame_size_select_matches_c() {
        assert_eq!(
            frame_size_select(OPUS_APPLICATION_AUDIO, 960, OPUS_FRAMESIZE_ARG, 48000),
            960
        );
        assert_eq!(
            frame_size_select(OPUS_APPLICATION_AUDIO, 960, OPUS_FRAMESIZE_20_MS, 48000),
            960
        );
        assert_eq!(
            frame_size_select(OPUS_APPLICATION_AUDIO, 480, OPUS_FRAMESIZE_20_MS, 48000),
            -1
        );
        assert_eq!(
            frame_size_select(OPUS_APPLICATION_AUDIO, 120, OPUS_FRAMESIZE_2_5_MS, 48000),
            120
        );
        assert_eq!(
            frame_size_select(OPUS_APPLICATION_AUDIO, 5760, OPUS_FRAMESIZE_120_MS, 48000),
            5760
        );
        assert_eq!(
            frame_size_select(OPUS_APPLICATION_AUDIO, 320, OPUS_FRAMESIZE_40_MS, 8000),
            320
        );
        assert_eq!(frame_size_select(OPUS_APPLICATION_AUDIO, 960, 12345, 48000), -1);
    }

    #[test]
    fn gen_toc_matches_c() {
        /* SILK-only, WB, 20 ms (framerate 50), mono: (1103-1101)<<5 | (3-2)<<3 = 0x48 */
        assert_eq!(gen_toc(MODE_SILK_ONLY, 50, OPUS_BANDWIDTH_WIDEBAND, 1), 0x48);
        /* SILK-only, NB, 20 ms, stereo: 0 | 8 | 4 = 12 */
        assert_eq!(gen_toc(MODE_SILK_ONLY, 50, OPUS_BANDWIDTH_NARROWBAND, 2), 12);
        /* CELT-only, FB, 20 ms: 0x80 | (1105-1102)<<5 | 3<<3 = 0xF8 */
        assert_eq!(gen_toc(MODE_CELT_ONLY, 50, OPUS_BANDWIDTH_FULLBAND, 1), 0xF8);
        /* Hybrid, SWB, 20 ms: 0x60 | 0<<4 | 1<<3 = 0x68 */
        assert_eq!(gen_toc(MODE_HYBRID, 50, OPUS_BANDWIDTH_SUPERWIDEBAND, 1), 0x68);
        /* Hybrid, FB, 20 ms: 0x60 | 1<<4 | 1<<3 = 0x78 */
        assert_eq!(gen_toc(MODE_HYBRID, 50, OPUS_BANDWIDTH_FULLBAND, 1), 0x78);
    }

    #[test]
    fn compute_equiv_rate_matches_c() {
        assert_eq!(compute_equiv_rate(64000, 2, 50, 1, 0, 10, 0), 64000 * 100 / 100);
        assert_eq!(compute_equiv_rate(64000, 2, 50, 0, 0, 10, 0), 64000 - 64000 / 12);
        /* complexity 4 in CELT-only mode: 64000*94/100 then *9/10 */
        assert_eq!(
            compute_equiv_rate(64000, 2, 50, 1, MODE_CELT_ONLY, 4, 0),
            64000 * 94 / 100 * 9 / 10
        );
    }

    #[test]
    fn compute_silk_rate_for_hybrid_matches_c() {
        let r = compute_silk_rate_for_hybrid(64000, OPUS_BANDWIDTH_FULLBAND, 1, 1, 0, 2);
        assert_eq!(r, 22000 * 2 - 1000);
    }

    #[test]
    fn user_bitrate_to_bitrate_matches_c() {
        let mut st = new_encoder(48000, 2, OPUS_APPLICATION_AUDIO);
        st.user_bitrate_bps = OPUS_AUTO;
        assert_eq!(user_bitrate_to_bitrate(&st, 960, 1276), 99000);
        st.user_bitrate_bps = OPUS_BITRATE_MAX;
        assert_eq!(user_bitrate_to_bitrate(&st, 960, 1276), 1500000.min(510400));
        st.user_bitrate_bps = 32000;
        assert_eq!(user_bitrate_to_bitrate(&st, 960, 1276), 32000);
    }

    #[test]
    fn create_and_ctl_round_trip() {
        let mut st = new_encoder(48000, 2, OPUS_APPLICATION_VOIP);
        let mut v = 0;
        assert_eq!(st.ctl(OPUS_GET_SAMPLE_RATE_REQUEST, &mut v), OPUS_OK);
        assert_eq!(v, 48000);
        assert_eq!(st.ctl(OPUS_GET_COMPLEXITY_REQUEST, &mut v), OPUS_OK);
        assert_eq!(v, 9);
        assert_eq!(st.ctl(OPUS_GET_BANDWIDTH_REQUEST, &mut v), OPUS_OK);
        assert_eq!(v, OPUS_BANDWIDTH_FULLBAND);
        assert_eq!(st.ctl(OPUS_GET_LOOKAHEAD_REQUEST, &mut v), OPUS_OK);
        assert_eq!(v, 48000 / 400 + 48000 / 250);

        let mut bad = -1;
        assert_eq!(st.ctl(OPUS_SET_COMPLEXITY_REQUEST, &mut bad), OPUS_BAD_ARG);
        let mut bad = 11;
        assert_eq!(st.ctl(OPUS_SET_COMPLEXITY_REQUEST, &mut bad), OPUS_BAD_ARG);
        let mut bad = OPUS_APPLICATION_RESTRICTED_SILK;
        assert_eq!(st.ctl(OPUS_SET_APPLICATION_REQUEST, &mut bad), OPUS_BAD_ARG);

        assert_eq!(st.ctl(999999, &mut v), OPUS_UNIMPLEMENTED);
    }

    #[test]
    fn create_rejects_bad_arguments() {
        let mut err = 0;
        assert!(OpusEncoder::create(44100, 2, OPUS_APPLICATION_AUDIO, &mut err).is_none());
        assert_eq!(err, OPUS_BAD_ARG);
        assert!(OpusEncoder::create(48000, 3, OPUS_APPLICATION_AUDIO, &mut err).is_none());
        assert!(OpusEncoder::create(48000, 2, 0, &mut err).is_none());
    }

    #[test]
    fn reset_state_restores_defaults() {
        let mut st = new_encoder(48000, 2, OPUS_APPLICATION_AUDIO);
        let mut pcm = vec![0i16; 960 * 2];
        for (i, v) in pcm.iter_mut().enumerate() {
            *v = ((i * 37) % 2000) as i16 - 1000;
        }
        let mut out = vec![0u8; 1500];
        assert!(st.encode(&pcm, 960, &mut out, 1500) > 0);
        assert_eq!(st.first, 0);
        let mut v = 0;
        assert_eq!(st.ctl(OPUS_RESET_STATE, &mut v), OPUS_OK);
        assert_eq!(st.first, 1);
        assert_eq!(st.mode, MODE_HYBRID);
        assert_eq!(st.bandwidth, OPUS_BANDWIDTH_FULLBAND);
        assert_eq!(st.stream_channels, 2);
        assert_eq!(st.rangeFinal, 0);
        assert_eq!(st.nb_no_activity_ms_Q1, 0);
    }

    #[test]
    fn decide_dtx_mode_bounds() {
        let mut ms = 0;
        for _ in 0..10 {
            assert_eq!(decide_dtx_mode(0, &mut ms, 40), 0);
        }
        assert_eq!(ms, 400);
        assert_eq!(decide_dtx_mode(0, &mut ms, 40), 1);
        assert_eq!(decide_dtx_mode(1, &mut ms, 40), 0);
        assert_eq!(ms, 0);
    }

    #[test]
    fn compute_quantizer_matches_c() {
        assert_eq!(compute_quantizer(6, 3, 15, 0), 6);
        assert_eq!(compute_quantizer(6, 3, 15, 1), 6 + (4 * 1 + 8) / 16);
        assert_eq!(compute_quantizer(15, 5, 15, 10), 15);
    }
}

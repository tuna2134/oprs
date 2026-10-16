//! Transliteration of `celt/celt_decoder.c` and of the `CELTDecoder` /
//! `OpusCustomDecoder` state from `celt/celt.h` (libopus 1.6.1) into safe Rust.
//!
//! Original C copyright notice (this file is a derived work of the
//! BSD-3-Clause libopus sources):
//
/* Copyright (c) 2007-2008 CSIRO
   Copyright (c) 2007-2010 Xiph.Org Foundation
   Copyright (c) 2008 Gregory Maxwell
   Written by Jean-Marc Valin and Gregory Maxwell */
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
//! # Configuration
//!
//! Float build (`FIXED_POINT`, `FLOAT_APPROX`, `ENABLE_QEXT`, `ENABLE_RES24`
//! undefined; `ENABLE_DEEP_PLC`, `ENABLE_DRED`, `ENABLE_OSCE`, `CUSTOM_MODES`
//! and `ENABLE_HARDENING` defined).  Every `#ifdef ENABLE_QEXT` block is
//! dropped; the `CUSTOM_MODES` blocks are kept.  The DNN itself (`dnn/`) is
//! out of scope (see `docs/progress.md`), so [`LPCNetPLCState`] is the minimal
//! decoder-visible view of `dnn/lpcnet_private.h` and the two DNN entry points
//! used by the PLC are documented no-ops; the neural PLC/DRED paths are
//! therefore *not* bit-exact (they are unreachable from a caller that passes
//! `lpcnet == None`, which is what [`celt_decode_with_ec`] does).
//!
//! # Slice conventions
//!
//! `st->mode` is owned (`Box<CELTMode>`) instead of borrowed; the arrays that C
//! reaches through `st->_decode_mem + …` (`oldBandE`, `oldLogE`, `oldLogE2`,
//! `backgroundLogE`, `lpc`) are separate `Vec`s.  `decode_mem` is one flat
//! buffer laid out exactly like the C allocation, channel `c` at offset
//! `c*(DECODE_BUFFER_SIZE+overlap)`.

use super::arch::*;
use super::bands::{anti_collapse, denormalise_bands, quant_all_bands, SPREAD_NORMAL};
use super::celt::{
    comb_filter, comb_filter_in_place, fromOpus, init_caps, resampling_factor, spread_icdf,
    tapset_icdf, tf_select_table, trim_icdf, COMBFILTER_MINPERIOD,
};
use super::celt_lpc::{_celt_autocorr, _celt_lpc, celt_fir, celt_iir, CELT_LPC_ORDER};
use super::entcode::*;
use super::entdec::*;
use super::mathops::{celt_lcg_rand, celt_sqrt, frac_div32};
use super::mdct::clt_mdct_backward;
use super::modes::{opus_custom_mode_create, CELTMode, DEC_PITCH_BUF_SIZE, MAX_PERIOD};
use super::pitch::{pitch_downsample, pitch_search};
use super::quant_bands::{unquant_coarse_energy, unquant_energy_finalise, unquant_fine_energy};
use super::rate::clt_compute_allocation;
use super::vq::renormalise_vector;
use crate::ffi;
use crate::types::*;

/// `OPUS_ARCHMASK` is 7, but `opus_select_arch()` returns 4 on the host the
/// reference configuration is pinned to (`docs/porting-conventions.md`); the
/// other ported modules pin it the same way.
pub const ARCH: i32 = 4;

/// `DECODE_BUFFER_SIZE` (`celt_decoder.c:79`) = `DEC_PITCH_BUF_SIZE`.
pub const DECODE_BUFFER_SIZE: i32 = DEC_PITCH_BUF_SIZE;

/// `PLC_PITCH_LAG_MAX` (`celt_decoder.c:62`).
pub const PLC_PITCH_LAG_MAX: i32 = 720;
/// `PLC_PITCH_LAG_MIN` (`celt_decoder.c:65`).
pub const PLC_PITCH_LAG_MIN: i32 = 100;

/// `FRAME_NONE`
pub const FRAME_NONE: i32 = 0;
/// `FRAME_NORMAL`
pub const FRAME_NORMAL: i32 = 1;
/// `FRAME_PLC_NOISE`
pub const FRAME_PLC_NOISE: i32 = 2;
/// `FRAME_PLC_PERIODIC`
pub const FRAME_PLC_PERIODIC: i32 = 3;
/// `FRAME_PLC_NEURAL`
pub const FRAME_PLC_NEURAL: i32 = 4;
/// `FRAME_DRED`
pub const FRAME_DRED: i32 = 5;

/// `PLC_UPDATE_FRAMES` (`celt_decoder.c:81`).
pub const PLC_UPDATE_FRAMES: i32 = 4;
/// `FRAME_SIZE` (`dnn/freq.h:42`, pulled in by `lpcnet.h`).
pub const FRAME_SIZE: i32 = 80 * 2;
/// `PLC_UPDATE_SAMPLES` (`celt_decoder.c:82`).
pub const PLC_UPDATE_SAMPLES: i32 = PLC_UPDATE_FRAMES * FRAME_SIZE;

/// `SIG_SAT` (`celt/arch.h:215`); `SATURATE()` is the identity in a float
/// build, so the value never affects the arithmetic.
const SIG_SAT: f32 = 536870911.0;

/// `sizeof(struct CELTDecoder)` in this configuration (float, no
/// `ENABLE_QEXT`), computed from the C source layout and validated by the
/// `celt_decoder` probe against `opus_custom_decoder_get_size()`.
const CELT_DECODER_STRUCT_SIZE: i32 = 1408;

/// `PREEMPHASIS` (`dnn/freq.h:34`).
const PREEMPHASIS: f32 = 0.85f32;

/// `MAXG()` (`celt/arch.h:110`).
#[inline(always)]
fn MAXG(a: f32, b: f32) -> f32 {
    MAX32(a, b)
}
/// `MING()` (`celt/arch.h:111`).
#[inline(always)]
fn MING(a: f32, b: f32) -> f32 {
    MIN32(a, b)
}

/// Minimal decoder-visible view of `LPCNetPLCState`
/// (`dnn/lpcnet_private.h`).
///
/// `dnn/` is out of scope for this port (`docs/progress.md`); only the fields
/// that `celt/celt_decoder.c` reads are modelled.  A caller that never passes
/// a state with `loaded != 0` never reaches the neural paths.
#[derive(Clone, Default)]
pub struct LPCNetPLCState {
    /// `int loaded`
    pub loaded: i32,
    /// `int fec_read_pos`
    pub fec_read_pos: i32,
    /// `int fec_fill_pos`
    pub fec_fill_pos: i32,
    /// `int fec_skip`
    pub fec_skip: i32,
    /// `int blend`
    pub blend: i32,
}

/// `lpcnet_plc_update()` (`dnn/lpcnet.h:172`).  The DNN is out of scope, so
/// this is a no-op; it is only reached on the neural-PLC path.
fn lpcnet_plc_update(_st: &mut LPCNetPLCState, _pcm: &[i16]) -> i32 {
    0
}

/// `lpcnet_plc_conceal()` (`dnn/lpcnet.h:174`).  The DNN is out of scope, so
/// this is a no-op; it is only reached on the neural-PLC path.
fn lpcnet_plc_conceal(_st: &mut LPCNetPLCState, _pcm: &mut [i16]) -> i32 {
    0
}

/* ------------------------------------------------------------------------- *
 * struct OpusCustomDecoder (celt_decoder.c:87)
 * ------------------------------------------------------------------------- */

/// `struct OpusCustomDecoder` / `CELTDecoder`.
pub struct CELTDecoder {
    /// `const OpusCustomMode *mode`
    pub mode: Box<CELTMode>,
    /// `int overlap`
    pub overlap: i32,
    /// `int channels`
    pub channels: i32,
    /// `int stream_channels`
    pub stream_channels: i32,

    /// `int downsample`
    pub downsample: i32,
    /// `int start`
    pub start: i32,
    /// `int end`
    pub end: i32,
    /// `int signalling`
    pub signalling: i32,
    /// `int disable_inv`
    pub disable_inv: i32,
    /// `int complexity`
    pub complexity: i32,
    /// `int arch`
    pub arch: i32,

    /* Everything beyond this point gets cleared on a reset
       (`#define DECODER_RESET_START rng`). */
    /// `opus_uint32 rng`
    pub rng: opus_uint32,
    /// `int error`
    pub error: i32,
    /// `int last_pitch_index`
    pub last_pitch_index: i32,
    /// `int loss_duration`
    pub loss_duration: i32,
    /// `int plc_duration`
    pub plc_duration: i32,
    /// `int last_frame_type`
    pub last_frame_type: i32,
    /// `int skip_plc`
    pub skip_plc: i32,
    /// `int postfilter_period`
    pub postfilter_period: i32,
    /// `int postfilter_period_old`
    pub postfilter_period_old: i32,
    /// `opus_val16 postfilter_gain`
    pub postfilter_gain: opus_val16,
    /// `opus_val16 postfilter_gain_old`
    pub postfilter_gain_old: opus_val16,
    /// `int postfilter_tapset`
    pub postfilter_tapset: i32,
    /// `int postfilter_tapset_old`
    pub postfilter_tapset_old: i32,
    /// `int prefilter_and_fold`
    pub prefilter_and_fold: i32,

    /// `celt_sig preemph_memD[2]`
    pub preemph_memD: [celt_sig; 2],

    /// `opus_int16 plc_pcm[PLC_UPDATE_SAMPLES]`
    pub plc_pcm: Vec<opus_int16>,
    /// `int plc_fill`
    pub plc_fill: i32,
    /// `float plc_preemphasis_mem`
    pub plc_preemphasis_mem: f32,

    /// `celt_sig _decode_mem[]`, `channels*(DECODE_BUFFER_SIZE+overlap)`
    pub decode_mem: Vec<celt_sig>,
    /// `celt_glog oldBandE[]`, `2*nbEBands`
    pub oldEBands: Vec<celt_glog>,
    /// `celt_glog oldLogE[]`, `2*nbEBands`
    pub oldLogE: Vec<celt_glog>,
    /// `celt_glog oldLogE2[]`, `2*nbEBands`
    pub oldLogE2: Vec<celt_glog>,
    /// `celt_glog backgroundLogE[]`, `2*nbEBands`
    pub backgroundLogE: Vec<celt_glog>,
    /// `opus_val16 lpc[]`, `channels*CELT_LPC_ORDER`
    pub lpc: Vec<opus_val16>,
}

impl Default for CELTDecoder {
    fn default() -> Self {
        CELTDecoder {
            mode: Box::new(CELTMode::default()),
            overlap: 0,
            channels: 0,
            stream_channels: 0,
            downsample: 1,
            start: 0,
            end: 0,
            signalling: 1,
            disable_inv: 0,
            complexity: 0,
            arch: ARCH,
            rng: 0,
            error: 0,
            last_pitch_index: 0,
            loss_duration: 0,
            plc_duration: 0,
            last_frame_type: FRAME_NONE,
            skip_plc: 0,
            postfilter_period: 0,
            postfilter_period_old: 0,
            postfilter_gain: 0.,
            postfilter_gain_old: 0.,
            postfilter_tapset: 0,
            postfilter_tapset_old: 0,
            prefilter_and_fold: 0,
            preemph_memD: [0.; 2],
            plc_pcm: Vec::new(),
            plc_fill: 0,
            plc_preemphasis_mem: 0.,
            decode_mem: Vec::new(),
            oldEBands: Vec::new(),
            oldLogE: Vec::new(),
            oldLogE2: Vec::new(),
            backgroundLogE: Vec::new(),
            lpc: Vec::new(),
        }
    }
}

impl CELTDecoder {
    /// An empty decoder; call [`celt_decoder_init`] or
    /// [`opus_custom_decoder_init`] before use (the C API allocates the state
    /// and then initialises it).
    pub fn new() -> CELTDecoder {
        CELTDecoder::default()
    }

    /// Byte offset of channel `c`'s plane in `decode_mem`.
    #[inline(always)]
    fn plane(&self, c: i32) -> i32 {
        c * (DECODE_BUFFER_SIZE + self.overlap)
    }

    /// `out_syn[c]` offset in `decode_mem` for a frame of `N` samples.
    #[inline(always)]
    fn out_syn(&self, c: i32, N: i32) -> i32 {
        self.plane(c) + DECODE_BUFFER_SIZE - N
    }
}

/* ------------------------------------------------------------------------- *
 * celt_decoder.c — size / init / create
 * ------------------------------------------------------------------------- */

/// `celt_decoder_get_size()` (`celt_decoder.c:181`).
pub fn celt_decoder_get_size(channels: i32) -> i32 {
    let mut error = 0;
    let mode = opus_custom_mode_create(48000, 960, &mut error)
        .expect("celt_decoder_get_size: the 48 kHz static mode always exists");
    opus_custom_decoder_get_size(&mode, channels)
}

/// `opus_custom_decoder_get_size()` (`celt_decoder.c:191`).
pub fn opus_custom_decoder_get_size(mode: &CELTMode, channels: i32) -> i32 {
    CELT_DECODER_STRUCT_SIZE
        + (channels * (DECODE_BUFFER_SIZE + mode.overlap) - 1) * 4
        + 4 * 2 * mode.nbEBands * 4
        + channels * CELT_LPC_ORDER * 4
}

/// `opus_custom_decoder_create()` (`celt_decoder.c:208`).
pub fn opus_custom_decoder_create(
    mode: Box<CELTMode>,
    channels: i32,
    error: &mut i32,
) -> Option<Box<CELTDecoder>> {
    let mut st = Box::new(CELTDecoder::default());
    let ret = opus_custom_decoder_init(&mut st, mode, channels);
    *error = ret;
    if ret != ffi::OPUS_OK {
        return None;
    }
    Some(st)
}

/// `opus_custom_decoder_destroy()` (`celt_decoder.c:278`).
///
/// The Rust port owns the state through a `Box`, so dropping it is the
/// deallocation; the function is kept for parity.
pub fn opus_custom_decoder_destroy(_st: Box<CELTDecoder>) {}

/// `celt_decoder_init()` (`celt_decoder.c:224`).
pub fn celt_decoder_init(st: &mut CELTDecoder, sampling_rate: opus_int32, channels: i32) -> i32 {
    let mut error = 0;
    let mode = opus_custom_mode_create(48000, 960, &mut error)
        .expect("celt_decoder_init: the 48 kHz static mode always exists");
    let ret = opus_custom_decoder_init(st, mode, channels);
    if ret != ffi::OPUS_OK {
        return ret;
    }
    st.downsample = resampling_factor(sampling_rate);
    if st.downsample == 0 {
        ffi::OPUS_BAD_ARG
    } else {
        ffi::OPUS_OK
    }
}

/// `opus_custom_decoder_init()` (`celt_decoder.c:242`).
pub fn opus_custom_decoder_init(st: &mut CELTDecoder, mode: Box<CELTMode>, channels: i32) -> i32 {
    if !(0..=2).contains(&channels) {
        return ffi::OPUS_BAD_ARG;
    }

    st.mode = mode;
    st.overlap = st.mode.overlap;
    st.stream_channels = channels;
    st.channels = channels;

    st.downsample = 1;
    st.start = 0;
    st.end = st.mode.effEBands;
    st.signalling = 1;
    st.disable_inv = (channels == 1) as i32;
    st.arch = ARCH;

    let mut dummy = 0;
    opus_custom_decoder_ctl(st, ffi::OPUS_RESET_STATE, &mut dummy);

    ffi::OPUS_OK
}

/// `validate_celt_decoder()` (`celt_decoder.c:144`), live because
/// `ENABLE_HARDENING` is defined.  The Rust asserts are `debug_assert!`, as
/// the reference is built without `ENABLE_ASSERTIONS`.
pub fn validate_celt_decoder(st: &CELTDecoder) {
    debug_assert!(st.end <= 25);
    debug_assert!(st.channels == 1 || st.channels == 2);
    debug_assert!(st.stream_channels == 1 || st.stream_channels == 2);
    debug_assert!(st.downsample > 0);
    debug_assert!(st.start == 0 || st.start == 17);
    debug_assert!(st.start < st.end);
    debug_assert!(st.arch >= 0);
    debug_assert!(st.arch <= 7);
    debug_assert!(st.last_pitch_index <= PLC_PITCH_LAG_MAX);
    debug_assert!(st.last_pitch_index >= PLC_PITCH_LAG_MIN || st.last_pitch_index == 0);
    debug_assert!(st.postfilter_period < MAX_PERIOD);
    debug_assert!(st.postfilter_period >= COMBFILTER_MINPERIOD || st.postfilter_period == 0);
    debug_assert!(st.postfilter_period_old < MAX_PERIOD);
    debug_assert!(
        st.postfilter_period_old >= COMBFILTER_MINPERIOD || st.postfilter_period_old == 0
    );
    debug_assert!(st.postfilter_tapset <= 2);
    debug_assert!(st.postfilter_tapset >= 0);
    debug_assert!(st.postfilter_tapset_old <= 2);
    debug_assert!(st.postfilter_tapset_old >= 0);
}

/// `opus_custom_decoder_ctl()` (`celt_decoder.c:1722`); `celt_decoder_ctl` is
/// the same function (`#define celt_decoder_ctl opus_custom_decoder_ctl`).
///
/// `value` is an in/out integer, as in the C: GET requests write their result
/// into it, SET requests read it.  `CELT_GET_MODE_REQUEST` has no integer
/// representation; read the public [`CELTDecoder::mode`] field instead (the
/// request itself returns `OPUS_UNIMPLEMENTED` here).
pub fn opus_custom_decoder_ctl(st: &mut CELTDecoder, request: i32, value: &mut i32) -> i32 {
    match request {
        ffi::OPUS_SET_COMPLEXITY_REQUEST => {
            if !(0..=10).contains(value) {
                return ffi::OPUS_BAD_ARG;
            }
            st.complexity = *value;
        }
        ffi::OPUS_GET_COMPLEXITY_REQUEST => {
            *value = st.complexity;
        }
        super::celt::CELT_SET_START_BAND_REQUEST => {
            if *value < 0 || *value >= st.mode.nbEBands {
                return ffi::OPUS_BAD_ARG;
            }
            st.start = *value;
        }
        super::celt::CELT_SET_END_BAND_REQUEST => {
            if *value < 1 || *value > st.mode.nbEBands {
                return ffi::OPUS_BAD_ARG;
            }
            st.end = *value;
        }
        super::celt::CELT_SET_CHANNELS_REQUEST => {
            if *value < 1 || *value > 2 {
                return ffi::OPUS_BAD_ARG;
            }
            st.stream_channels = *value;
        }
        super::celt::CELT_GET_AND_CLEAR_ERROR_REQUEST => {
            *value = st.error;
            st.error = 0;
        }
        ffi::OPUS_GET_LOOKAHEAD_REQUEST => {
            *value = st.overlap / st.downsample;
        }
        ffi::OPUS_RESET_STATE => {
            decoder_reset_state(st);
        }
        ffi::OPUS_GET_PITCH_REQUEST => {
            *value = st.postfilter_period;
        }
        super::celt::CELT_SET_SIGNALLING_REQUEST => {
            st.signalling = *value;
        }
        ffi::OPUS_GET_FINAL_RANGE_REQUEST => {
            *value = st.rng as i32;
        }
        ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST => {
            if *value < 0 || *value > 1 {
                return ffi::OPUS_BAD_ARG;
            }
            st.disable_inv = *value;
        }
        ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST => {
            *value = st.disable_inv;
        }
        _ => {
            return ffi::OPUS_UNIMPLEMENTED;
        }
    }
    ffi::OPUS_OK
}

/// `celt_decoder_ctl` (`#define celt_decoder_ctl opus_custom_decoder_ctl`).
pub fn celt_decoder_ctl(st: &mut CELTDecoder, request: i32, value: &mut i32) -> i32 {
    opus_custom_decoder_ctl(st, request, value)
}

/// The `OPUS_RESET_STATE` body: `OPUS_CLEAR()` of everything from
/// `DECODER_RESET_START` onwards, followed by the `oldLogE`/`oldLogE2` fill
/// and the two extra fields.
fn decoder_reset_state(st: &mut CELTDecoder) {
    let nbEBands = st.mode.nbEBands;
    let channels = st.channels;
    let overlap = st.overlap;

    st.rng = 0;
    st.error = 0;
    st.last_pitch_index = 0;
    st.loss_duration = 0;
    st.plc_duration = 0;
    st.last_frame_type = 0;
    st.skip_plc = 0;
    st.postfilter_period = 0;
    st.postfilter_period_old = 0;
    st.postfilter_gain = 0.;
    st.postfilter_gain_old = 0.;
    st.postfilter_tapset = 0;
    st.postfilter_tapset_old = 0;
    st.prefilter_and_fold = 0;
    st.preemph_memD = [0.; 2];

    st.plc_pcm = vec![0 as opus_int16; PLC_UPDATE_SAMPLES as usize];
    st.plc_fill = 0;
    st.plc_preemphasis_mem = 0.;

    st.decode_mem = vec![0 as celt_sig; (channels * (DECODE_BUFFER_SIZE + overlap)) as usize];
    st.oldEBands = vec![0 as celt_glog; (2 * nbEBands) as usize];
    st.oldLogE = vec![-GCONST(28.0f32); (2 * nbEBands) as usize];
    st.oldLogE2 = vec![-GCONST(28.0f32); (2 * nbEBands) as usize];
    st.backgroundLogE = vec![0 as celt_glog; (2 * nbEBands) as usize];
    st.lpc = vec![0 as opus_val16; (channels * CELT_LPC_ORDER) as usize];

    st.skip_plc = 1;
    st.last_frame_type = FRAME_NONE;
}

/* ------------------------------------------------------------------------- *
 * deemphasis() (celt_decoder.c:318)
 * ------------------------------------------------------------------------- */

/// `deemphasis()` (`celt_decoder.c:318`).
///
/// `inp[c]` is the C `in[c]` pointer (already offset to `out_syn`); `pcm` is
/// interleaved `[N*C]`.  The `deemphasis_stereo_simple()` shortcut is *not*
/// compiled in this configuration (it is guarded by `!CUSTOM_MODES`).
pub fn deemphasis(
    inp: &[&[celt_sig]],
    pcm: &mut [opus_res],
    N: i32,
    C: i32,
    downsample: i32,
    coef: &[opus_val16],
    mem: &mut [celt_sig],
    accum: i32,
) {
    let mut scratch = vec![0 as celt_sig; N as usize];
    let coef0 = coef[0];
    let Nd = N / downsample;
    let mut c = 0;
    loop {
        let mut apply_downsampling = 0;
        let mut m = mem[c as usize];
        let x = inp[c as usize];
        /* `coef[1] != 0` is live because `CUSTOM_MODES` is defined; for the
           standard 48 kHz mode the coefficient is zero, so the `downsample>1`
           shortcut is taken instead. */
        if coef[1] != 0. {
            let coef1 = coef[1];
            let coef3 = coef[3];
            for j in 0..N as usize {
                let mut tmp = SATURATE(x[j] + m + VERY_SMALL, SIG_SAT);
                m = MULT16_32_Q15(coef0, tmp) - MULT16_32_Q15(coef1, x[j]);
                tmp = SHL32(MULT16_32_Q15(coef3, tmp), 2);
                scratch[j] = tmp;
            }
            apply_downsampling = 1;
        } else if downsample > 1 {
            for j in 0..N as usize {
                let tmp = SATURATE(x[j] + VERY_SMALL + m, SIG_SAT);
                m = MULT16_32_Q15(coef0, tmp);
                scratch[j] = tmp;
            }
            apply_downsampling = 1;
        } else if accum != 0 {
            for j in 0..N as usize {
                let tmp = SATURATE(x[j] + m + VERY_SMALL, SIG_SAT);
                m = MULT16_32_Q15(coef0, tmp);
                let k = j * C as usize + c as usize;
                pcm[k] = ADD_RES(pcm[k], SIG2RES(tmp));
            }
        } else {
            for j in 0..N as usize {
                let tmp = SATURATE(x[j] + VERY_SMALL + m, SIG_SAT);
                m = MULT16_32_Q15(coef0, tmp);
                pcm[j * C as usize + c as usize] = SIG2RES(tmp);
            }
        }
        mem[c as usize] = m;

        if apply_downsampling != 0 {
            if accum != 0 {
                for j in 0..Nd as usize {
                    let k = j * C as usize + c as usize;
                    pcm[k] = ADD_RES(pcm[k], SIG2RES(scratch[j * downsample as usize]));
                }
            } else {
                for j in 0..Nd as usize {
                    pcm[j * C as usize + c as usize] =
                        SIG2RES(scratch[j * downsample as usize]);
                }
            }
        }
        c += 1;
        if c >= C {
            break;
        }
    }
}

/* ------------------------------------------------------------------------- *
 * celt_synthesis() (celt_decoder.c:413)
 * ------------------------------------------------------------------------- */

/// `celt_synthesis()` (`celt_decoder.c:413`).
///
/// `out` is the flat `decode_mem`; `out_off[c]` is `out_syn[c]`.  The C
/// aliasing trick (using the output buffer as `freq2`) is replaced by an owned
/// copy, which is observationally identical because the aliased region is only
/// read *before* the MDCT writes that would clobber it.
pub fn celt_synthesis(
    mode: &CELTMode,
    X: &mut [celt_norm],
    out: &mut [celt_sig],
    out_off: [i32; 2],
    oldBandE: &[celt_glog],
    start: i32,
    effEnd: i32,
    C: i32,
    CC: i32,
    isTransient: i32,
    LM: i32,
    downsample: i32,
    silence: i32,
    arch: i32,
) {
    let overlap = mode.overlap;
    let nbEBands = mode.nbEBands;
    let N = mode.shortMdctSize << LM;
    let mut freq = vec![0 as celt_sig; N as usize]; /* Interleaved signal MDCTs */
    let M = 1 << LM;

    let B: i32;
    let NB: i32;
    let shift: i32;
    if isTransient != 0 {
        B = M;
        NB = mode.shortMdctSize;
        shift = mode.maxLM;
    } else {
        B = 1;
        NB = mode.shortMdctSize << LM;
        shift = mode.maxLM - LM;
    }

    if CC == 2 && C == 1 {
        /* Copying a mono stream to two channels.  The C stores a temporary
           copy in `out_syn[1]+overlap/2` because the IMDCT destroys its
           input; an owned buffer is equivalent (see the doc comment). */
        denormalise_bands(
            mode, X, &mut freq, oldBandE, start, effEnd, M, downsample, silence,
        );
        let mut freq2 = vec![0 as celt_sig; N as usize];
        freq2.copy_from_slice(&freq);
        for b in 0..B {
            let off = (out_off[0] + NB * b) as usize;
            clt_mdct_backward(
                &mode.mdct,
                &freq2[b as usize..],
                &mut out[off..],
                &mode.window,
                overlap,
                shift,
                B,
                arch,
            );
        }
        for b in 0..B {
            let off = (out_off[1] + NB * b) as usize;
            clt_mdct_backward(
                &mode.mdct,
                &freq[b as usize..],
                &mut out[off..],
                &mode.window,
                overlap,
                shift,
                B,
                arch,
            );
        }
    } else if CC == 1 && C == 2 {
        /* Downmixing a stereo stream to mono. */
        denormalise_bands(
            mode, X, &mut freq, oldBandE, start, effEnd, M, downsample, silence,
        );
        let mut freq2 = vec![0 as celt_sig; N as usize];
        denormalise_bands(
            mode,
            &X[N as usize..],
            &mut freq2,
            &oldBandE[nbEBands as usize..],
            start,
            effEnd,
            M,
            downsample,
            silence,
        );
        for i in 0..N as usize {
            freq[i] = ADD32(HALF32(freq[i]), HALF32(freq2[i]));
        }
        for b in 0..B {
            let off = (out_off[0] + NB * b) as usize;
            clt_mdct_backward(
                &mode.mdct,
                &freq[b as usize..],
                &mut out[off..],
                &mode.window,
                overlap,
                shift,
                B,
                arch,
            );
        }
    } else {
        /* Normal case (mono or stereo). */
        let mut c = 0;
        loop {
            denormalise_bands(
                mode,
                &X[(c * N) as usize..],
                &mut freq,
                &oldBandE[(c * nbEBands) as usize..],
                start,
                effEnd,
                M,
                downsample,
                silence,
            );
            for b in 0..B {
                let off = (out_off[c as usize] + NB * b) as usize;
                clt_mdct_backward(
                    &mode.mdct,
                    &freq[b as usize..],
                    &mut out[off..],
                    &mode.window,
                    overlap,
                    shift,
                    B,
                    arch,
                );
            }
            c += 1;
            if c >= CC {
                break;
            }
        }
    }
    /* Saturate IMDCT output so that we can't overflow in the pitch postfilter. */
    let mut c = 0;
    loop {
        let base = out_off[c as usize] as usize;
        for i in 0..N as usize {
            out[base + i] = SATURATE(out[base + i], SIG_SAT);
        }
        c += 1;
        if c >= CC {
            break;
        }
    }
}

/* ------------------------------------------------------------------------- *
 * tf_decode() (celt_decoder.c:513)
 * ------------------------------------------------------------------------- */

/// `tf_decode()` (`celt_decoder.c:513`).
fn tf_decode(
    start: i32,
    end: i32,
    isTransient: i32,
    tf_res: &mut [i32],
    LM: i32,
    dec: &mut ec_ctx,
) {
    let budget0: opus_uint32 = dec.storage.wrapping_mul(8);
    let mut tell: opus_uint32 = dec.ec_tell() as opus_uint32;
    let mut logp: i32 = if isTransient != 0 { 2 } else { 4 };
    let tf_select_rsv: i32 =
        (LM > 0 && tell.wrapping_add(logp as u32).wrapping_add(1) <= budget0) as i32;
    let budget: opus_uint32 = budget0.wrapping_sub(tf_select_rsv as opus_uint32);
    let mut tf_changed: i32 = 0;
    let mut curr: i32 = 0;
    for i in start..end {
        if tell.wrapping_add(logp as u32) <= budget {
            curr ^= ec_dec_bit_logp(dec, logp as u32);
            tell = dec.ec_tell() as opus_uint32;
            tf_changed |= curr;
        }
        tf_res[i as usize] = curr;
        logp = if isTransient != 0 { 4 } else { 5 };
    }
    let mut tf_select: i32 = 0;
    if tf_select_rsv != 0
        && tf_select_table[LM as usize][(4 * isTransient + 0 + tf_changed) as usize]
            != tf_select_table[LM as usize][(4 * isTransient + 2 + tf_changed) as usize]
    {
        tf_select = ec_dec_bit_logp(dec, 1);
    }
    for i in start..end {
        tf_res[i as usize] = tf_select_table[LM as usize]
            [(4 * isTransient + 2 * tf_select + tf_res[i as usize]) as usize]
            as i32;
    }
}

/* ------------------------------------------------------------------------- *
 * celt_plc_pitch_search() (celt_decoder.c:552)
 * ------------------------------------------------------------------------- */

/// `celt_plc_pitch_search()` (`celt_decoder.c:552`).
fn celt_plc_pitch_search(st: &CELTDecoder, C: i32, arch: i32) -> i32 {
    let mut lp_pitch_buf = vec![0 as opus_val16; (DECODE_BUFFER_SIZE >> 1) as usize];
    let p0 = st.plane(0) as usize;
    let p1 = if C == 2 { st.plane(1) as usize } else { p0 };
    let buf0 = &st.decode_mem[p0..p0 + DECODE_BUFFER_SIZE as usize];
    let buf1 = &st.decode_mem[p1..p1 + DECODE_BUFFER_SIZE as usize];
    let planes: [&[celt_sig]; 2] = [buf0, buf1];
    pitch_downsample(
        &planes[..C as usize],
        &mut lp_pitch_buf,
        DECODE_BUFFER_SIZE >> 1,
        C,
        2,
        arch,
    );
    let mut pitch_index = 0;
    pitch_search(
        &lp_pitch_buf[(PLC_PITCH_LAG_MAX >> 1) as usize..],
        &lp_pitch_buf,
        DECODE_BUFFER_SIZE - PLC_PITCH_LAG_MAX,
        PLC_PITCH_LAG_MAX - PLC_PITCH_LAG_MIN,
        &mut pitch_index,
        arch,
    );
    PLC_PITCH_LAG_MAX - pitch_index
}

/* ------------------------------------------------------------------------- *
 * prefilter_and_fold() (celt_decoder.c:576)
 * ------------------------------------------------------------------------- */

/// `prefilter_and_fold()` (`celt_decoder.c:576`).
fn prefilter_and_fold(st: &mut CELTDecoder, N: i32) {
    let overlap = st.overlap;
    let CC = st.channels;
    let mut etmp = vec![0 as opus_val32; overlap as usize];
    let mut c = 0;
    loop {
        let plane = st.plane(c);
        let T0 = st.postfilter_period_old;
        let T1 = st.postfilter_period;
        let h = IMAX(IMAX(T0, T1), COMBFILTER_MINPERIOD) + 2;
        let xstart = (plane + DECODE_BUFFER_SIZE - N - h) as usize;
        comb_filter(
            &mut etmp,
            &st.decode_mem[xstart..],
            T0,
            T1,
            overlap,
            -st.postfilter_gain_old,
            -st.postfilter_gain,
            st.postfilter_tapset_old,
            st.postfilter_tapset,
            &[],
            0,
            st.arch,
        );
        for i in 0..overlap / 2 {
            let dst = (plane + DECODE_BUFFER_SIZE - N + i) as usize;
            st.decode_mem[dst] = MULT16_32_Q15(
                COEF2VAL16(st.mode.window[i as usize]),
                etmp[(overlap - 1 - i) as usize],
            ) + MULT16_32_Q15(
                COEF2VAL16(st.mode.window[(overlap - i - 1) as usize]),
                etmp[i as usize],
            );
        }
        c += 1;
        if c >= CC {
            break;
        }
    }
}

/* ------------------------------------------------------------------------- *
 * ENABLE_DEEP_PLC helpers (celt_decoder.c:623)
 * ------------------------------------------------------------------------- */

const SINC_ORDER: i32 = 48;

/// `sinc_filter[SINC_ORDER+1]` (`celt_decoder.c:629`).
#[allow(non_upper_case_globals)]
static sinc_filter: [f32; (SINC_ORDER + 1) as usize] = [
    4.2931e-05, -0.000190293, -0.000816132, -0.000637162, 0.00141662, 0.00354764, 0.00184368,
    -0.00428274, -0.00856105, -0.0034003, 0.00930201, 0.0159616, 0.00489785, -0.0169649,
    -0.0259484, -0.00596856, 0.0286551, 0.0405872, 0.00649994, -0.0509284, -0.0716655,
    -0.00665212, 0.134336, 0.278927, 0.339995, 0.278927, 0.134336, -0.00665212, -0.0716655,
    -0.0509284, 0.00649994, 0.0405872, 0.0286551, -0.00596856, -0.0259484, -0.0169649, 0.00489785,
    0.0159616, 0.00930201, -0.0034003, -0.00856105, -0.00428274, 0.00184368, 0.00354764, 0.00141662,
    -0.000637162, -0.000816132, -0.000190293, 4.2931e-05,
];

/// `update_plc_state()` (`celt_decoder.c:639`).
///
/// The two `lpcnet_plc_*` calls are no-ops (DNN out of scope).
/// `decode_mem` is the flat C `_decode_mem`; `stride` is
/// `DECODE_BUFFER_SIZE+overlap`.
fn update_plc_state(
    lpcnet: &mut LPCNetPLCState,
    decode_mem: &[celt_sig],
    stride: i32,
    plc_preemphasis_mem: &mut f32,
    CC: i32,
) {
    let mut buf48k = vec![0 as celt_sig; DECODE_BUFFER_SIZE as usize];
    if CC == 1 {
        buf48k.copy_from_slice(&decode_mem[0..DECODE_BUFFER_SIZE as usize]);
    } else {
        for i in 0..DECODE_BUFFER_SIZE as usize {
            buf48k[i] = 0.5 * (decode_mem[i] + decode_mem[stride as usize + i]);
        }
    }
    /* Down-sample the last 40 ms. */
    for i in 1..DECODE_BUFFER_SIZE as usize {
        buf48k[i] += PREEMPHASIS * buf48k[i - 1];
    }
    *plc_preemphasis_mem = buf48k[(DECODE_BUFFER_SIZE - 1) as usize];
    let offset = DECODE_BUFFER_SIZE - SINC_ORDER - 1 - 3 * (PLC_UPDATE_SAMPLES - 1);
    debug_assert!(3 * (PLC_UPDATE_SAMPLES - 1) + SINC_ORDER + offset == DECODE_BUFFER_SIZE - 1);
    let mut buf16k = vec![0 as opus_int16; PLC_UPDATE_SAMPLES as usize];
    for i in 0..PLC_UPDATE_SAMPLES as usize {
        let mut sum = 0f32;
        for j in 0..(SINC_ORDER + 1) as usize {
            sum += buf48k[3 * i + j + offset as usize] * sinc_filter[j];
        }
        buf16k[i] = float2int(MIN32(32767.0, MAX32(-32767.0, sum))) as opus_int16;
    }
    let tmp_read_post = lpcnet.fec_read_pos;
    let tmp_fec_skip = lpcnet.fec_skip;
    for i in 0..PLC_UPDATE_FRAMES {
        lpcnet_plc_update(lpcnet, &buf16k[(FRAME_SIZE * i) as usize..]);
    }
    lpcnet.fec_read_pos = tmp_read_post;
    lpcnet.fec_skip = tmp_fec_skip;
}

/* ------------------------------------------------------------------------- *
 * celt_decode_lost() (celt_decoder.c:675)
 * ------------------------------------------------------------------------- */

/// `celt_decode_lost()` (`celt_decoder.c:675`).
fn celt_decode_lost(
    st: &mut CELTDecoder,
    N: i32,
    LM: i32,
    mut lpcnet: Option<&mut LPCNetPLCState>,
) {
    let C = st.channels;
    let nbEBands = st.mode.nbEBands;
    let overlap = st.mode.overlap;
    let eBands = st.mode.eBands.clone();
    let max_period = MAX_PERIOD;
    let decode_buffer_size = DECODE_BUFFER_SIZE;

    let loss_duration = st.loss_duration;
    let start = st.start;
    let mut curr_frame_type = FRAME_PLC_PERIODIC;
    if st.plc_duration >= 40 || start != 0 || st.skip_plc != 0 {
        curr_frame_type = FRAME_PLC_NOISE;
    }
    /* ENABLE_DEEP_PLC: `start == 0 && lpcnet != NULL && Fs != 96000 && loaded` */
    if start == 0 {
        if let Some(ref l) = lpcnet {
            if st.mode.Fs != 96000 && l.loaded != 0 {
                if st.complexity >= 5 && st.plc_duration < 80 && st.skip_plc == 0 {
                    curr_frame_type = FRAME_PLC_NEURAL;
                }
                /* ENABLE_DRED */
                if l.fec_fill_pos > l.fec_read_pos {
                    curr_frame_type = FRAME_DRED;
                }
            }
        }
    }

    if curr_frame_type == FRAME_PLC_NOISE {
        /* Noise-based PLC/CNG */
        let mut X = vec![0 as celt_norm; (C * N) as usize];
        let end = st.end;
        let effEnd = IMAX(start, IMIN(end, st.mode.effEBands));

        let mut c = 0;
        loop {
            let plane = st.plane(c) as usize;
            let n = N as usize;
            let total = (decode_buffer_size + overlap) as usize;
            st.decode_mem[plane..plane + total].copy_within(n..total, 0);
            c += 1;
            if c >= C {
                break;
            }
        }

        if st.prefilter_and_fold != 0 {
            prefilter_and_fold(st, N);
        }

        /* Energy decay */
        let decay = GCONST(if loss_duration == 0 { 1.5f32 } else { 0.5f32 });
        let mut c = 0;
        loop {
            for i in start..end {
                let idx = (c * nbEBands + i) as usize;
                st.oldEBands[idx] = MAXG(st.backgroundLogE[idx], st.oldEBands[idx] - decay);
            }
            c += 1;
            if c >= C {
                break;
            }
        }
        let mut seed = st.rng;
        for c in 0..C {
            for i in start..effEnd {
                let boffs = N * c + ((eBands[i as usize] as i32) << LM);
                let blen =
                    ((eBands[(i + 1) as usize] as i32) - (eBands[i as usize] as i32)) << LM;
                for j in 0..blen {
                    seed = celt_lcg_rand(seed);
                    X[(boffs + j) as usize] =
                        SHL32(((seed as i32) >> 20) as celt_norm, NORM_SHIFT - 14);
                }
                let b = boffs as usize;
                renormalise_vector(&mut X[b..b + blen as usize], blen, Q31ONE, st.arch);
            }
        }
        st.rng = seed;

        let out_off = [st.out_syn(0, N), st.out_syn(1, N)];
        celt_synthesis(
            &st.mode,
            &mut X,
            &mut st.decode_mem,
            out_off,
            &st.oldEBands,
            start,
            effEnd,
            C,
            C,
            0,
            LM,
            st.downsample,
            0,
            st.arch,
        );

        /* Run the postfilter with the last parameters. */
        let mut c = 0;
        loop {
            st.postfilter_period = IMAX(st.postfilter_period, COMBFILTER_MINPERIOD);
            st.postfilter_period_old = IMAX(st.postfilter_period_old, COMBFILTER_MINPERIOD);
            let plane = st.plane(c) as usize;
            let plane_end = plane + (decode_buffer_size + overlap) as usize;
            let x_off = decode_buffer_size - N;
            let T0 = st.postfilter_period_old;
            let T1 = st.postfilter_period;
            let g0 = st.postfilter_gain_old;
            let g1 = st.postfilter_gain;
            let t0 = st.postfilter_tapset_old;
            let t1 = st.postfilter_tapset;
            let shortMdctSize = st.mode.shortMdctSize;
            comb_filter_in_place(
                &mut st.decode_mem[plane..plane_end],
                x_off,
                T0,
                T1,
                shortMdctSize,
                g0,
                g1,
                t0,
                t1,
                &st.mode.window,
                overlap,
                st.arch,
            );
            if LM != 0 {
                comb_filter_in_place(
                    &mut st.decode_mem[plane..plane_end],
                    x_off + shortMdctSize,
                    st.postfilter_period,
                    st.postfilter_period,
                    N - shortMdctSize,
                    st.postfilter_gain,
                    st.postfilter_gain,
                    st.postfilter_tapset,
                    st.postfilter_tapset,
                    &st.mode.window,
                    overlap,
                    st.arch,
                );
            }
            c += 1;
            if c >= C {
                break;
            }
        }
        st.postfilter_period_old = st.postfilter_period;
        st.postfilter_gain_old = st.postfilter_gain;
        st.postfilter_tapset_old = st.postfilter_tapset;

        st.prefilter_and_fold = 0;
        /* Skip regular PLC until we get two consecutive packets. */
        st.skip_plc = 1;
    } else {
        /* Pitch-based PLC */
        let window = st.mode.window.clone();
        let mut fade = Q15ONE;
        let pitch_index;
        let curr_neural = curr_frame_type == FRAME_PLC_NEURAL || curr_frame_type == FRAME_DRED;
        let last_neural =
            st.last_frame_type == FRAME_PLC_NEURAL || st.last_frame_type == FRAME_DRED;
        if st.last_frame_type != FRAME_PLC_PERIODIC && !(last_neural && curr_neural) {
            pitch_index = celt_plc_pitch_search(st, C, st.arch);
            st.last_pitch_index = pitch_index;
        } else {
            pitch_index = st.last_pitch_index;
            fade = QCONST16(0.8f32, 15);
        }
        /* ENABLE_DEEP_PLC */
        if curr_neural && !last_neural {
            if let Some(ln) = lpcnet.as_deref_mut() {
                let stride = decode_buffer_size + overlap;
                update_plc_state(ln, &st.decode_mem, stride, &mut st.plc_preemphasis_mem, C);
            }
        }

        /* We want the excitation for 2 pitch periods in order to look for a
           decaying signal, but we can't get more than MAX_PERIOD. */
        let exc_length = IMIN(2 * pitch_index, max_period);

        let mut _exc = vec![0 as opus_val16; (max_period + CELT_LPC_ORDER) as usize];
        let mut fir_tmp = vec![0 as opus_val16; exc_length as usize];
        let exc_off = CELT_LPC_ORDER;

        let mut c = 0;
        loop {
            let decay: opus_val16;
            let mut S1: opus_val32 = 0.;
            let plane = st.plane(c);
            let bufbase = plane as usize;

            for i in 0..(max_period + CELT_LPC_ORDER) as usize {
                let src =
                    bufbase + (decode_buffer_size - max_period - CELT_LPC_ORDER) as usize + i;
                _exc[i] = SROUND16(st.decode_mem[src], SIG_SHIFT);
            }

            if st.last_frame_type != FRAME_PLC_PERIODIC && !(last_neural && curr_neural) {
                let mut ac = [0 as opus_val32; (CELT_LPC_ORDER + 1) as usize];
                /* Compute LPC coefficients for the last MAX_PERIOD samples
                   before the first loss so we can work in the excitation
                   domain. */
                _celt_autocorr(
                    &_exc[exc_off as usize..],
                    &mut ac,
                    Some(&window),
                    overlap,
                    CELT_LPC_ORDER,
                    max_period,
                    st.arch,
                );
                /* Add a noise floor of -40 dB. */
                ac[0] *= 1.0001f32;
                /* Use lag windowing to stabilize the Levinson-Durbin recursion. */
                for i in 1..=CELT_LPC_ORDER {
                    ac[i as usize] -=
                        ac[i as usize] * (0.008f32 * 0.008f32) * (i as f32) * (i as f32);
                }
                let lo = (c * CELT_LPC_ORDER) as usize;
                let hi = lo + CELT_LPC_ORDER as usize;
                _celt_lpc(&mut st.lpc[lo..hi], &ac, CELT_LPC_ORDER);
            }
            /* Initialize the LPC history with the samples just before the
               start of the region for which we're computing the excitation.
               The C `celt_fir()` cannot filter in-place, hence the copy. */
            {
                let a = (max_period - exc_length) as usize;
                let lo = (c * CELT_LPC_ORDER) as usize;
                let hi = lo + CELT_LPC_ORDER as usize;
                celt_fir(
                    &_exc[a..],
                    &st.lpc[lo..hi],
                    &mut fir_tmp,
                    exc_length,
                    CELT_LPC_ORDER,
                    st.arch,
                );
                let dst = exc_off as usize + a;
                _exc[dst..dst + exc_length as usize]
                    .copy_from_slice(&fir_tmp[..exc_length as usize]);
            }

            /* Check if the waveform is decaying, and if so how fast.  We do
               this to avoid adding energy when concealing in a segment with
               decaying energy. */
            {
                let mut E1: opus_val32 = 1.;
                let mut E2: opus_val32 = 1.;
                let decay_length = exc_length >> 1;
                for i in 0..decay_length {
                    let mut e =
                        _exc[(exc_off + max_period - decay_length + i) as usize];
                    E1 += SHR32(MULT16_16(e, e), 0);
                    e = _exc[(exc_off + max_period - 2 * decay_length + i) as usize];
                    E2 += SHR32(MULT16_16(e, e), 0);
                }
                E1 = MIN32(E1, E2);
                decay = celt_sqrt(frac_div32(SHR32(E1, 1), E2));
            }

            /* Move the decoder memory one frame to the left to give us room
               to add the data for the new frame. */
            let n = N as usize;
            let total = (decode_buffer_size + overlap) as usize;
            st.decode_mem[bufbase..bufbase + total].copy_within(n..total, 0);

            /* Extrapolate from the end of the excitation with a period of
               "pitch_index", scaling down each period by an additional factor
               of "decay". */
            let extrapolation_offset = max_period - pitch_index;
            /* We need to extrapolate enough samples to cover a complete MDCT
               window (including overlap/2 samples on both sides). */
            let extrapolation_len = N + overlap;
            /* We also apply fading if this is not the first loss. */
            let mut attenuation = MULT16_16_Q15(fade, decay);
            let mut i: i32 = 0;
            let mut j: i32 = 0;
            while i < extrapolation_len {
                if j >= pitch_index {
                    j -= pitch_index;
                    attenuation = MULT16_16_Q15(attenuation, decay);
                }
                let src = (exc_off + extrapolation_offset + j) as usize;
                let dst = bufbase + (decode_buffer_size - N + i) as usize;
                st.decode_mem[dst] =
                    SHL32(EXTEND32(MULT16_16_Q15(attenuation, _exc[src])), SIG_SHIFT);
                /* Compute the energy of the previously decoded signal whose
                   excitation we're copying. */
                let tmp = SROUND16(
                    st.decode_mem[bufbase
                        + (decode_buffer_size - max_period - N + extrapolation_offset + j)
                            as usize],
                    SIG_SHIFT,
                );
                S1 += SHR32(MULT16_16(tmp, tmp), 11);
                i += 1;
                j += 1;
            }
            {
                let mut lpc_mem = [0 as opus_val16; CELT_LPC_ORDER as usize];
                /* Copy the last decoded samples (prior to the overlap region)
                   to synthesis filter memory so we can have a continuous
                   signal. */
                for i in 0..CELT_LPC_ORDER {
                    lpc_mem[i as usize] = SROUND16(
                        st.decode_mem[bufbase + (decode_buffer_size - N - 1 - i) as usize],
                        SIG_SHIFT,
                    );
                }
                /* Apply the synthesis filter to convert the excitation back
                   into the signal domain.  The C aliases `_x` and `_y`; the
                   Rust `celt_iir` takes them separately, so `_x` is copied
                   first (the arithmetic is unaffected). */
                let lo = bufbase + (decode_buffer_size - N) as usize;
                let hi = lo + extrapolation_len as usize;
                let xcopy = st.decode_mem[lo..hi].to_vec();
                let lpc_lo = (c * CELT_LPC_ORDER) as usize;
                let lpc_hi = lpc_lo + CELT_LPC_ORDER as usize;
                celt_iir(
                    &xcopy,
                    &st.lpc[lpc_lo..lpc_hi],
                    &mut st.decode_mem[lo..hi],
                    extrapolation_len,
                    CELT_LPC_ORDER,
                    &mut lpc_mem,
                    st.arch,
                );
            }

            /* Check if the synthesis energy is higher than expected, which
               can happen with the signal changes during our window.  If so,
               attenuate. */
            {
                let mut S2: opus_val32 = 0.;
                for i in 0..extrapolation_len {
                    let tmp = SROUND16(
                        st.decode_mem[bufbase + (decode_buffer_size - N + i) as usize],
                        SIG_SHIFT,
                    );
                    S2 += SHR32(MULT16_16(tmp, tmp), 11);
                }
                /* The float test is written this way to catch NaNs in the
                   output of the IIR filter at the same time. */
                if !(S1 > 0.2f32 * S2) {
                    for i in 0..extrapolation_len {
                        st.decode_mem[bufbase + (decode_buffer_size - N + i) as usize] = 0.;
                    }
                } else if S1 < S2 {
                    let ratio = celt_sqrt(frac_div32(SHR32(S1, 1) + 1., S2 + 1.));
                    for i in 0..overlap {
                        let tmp_g =
                            Q15ONE - MULT16_16_Q15(COEF2VAL16(window[i as usize]), Q15ONE - ratio);
                        let k = bufbase + (decode_buffer_size - N + i) as usize;
                        st.decode_mem[k] = MULT16_32_Q15(tmp_g, st.decode_mem[k]);
                    }
                    for i in overlap..extrapolation_len {
                        let k = bufbase + (decode_buffer_size - N + i) as usize;
                        st.decode_mem[k] = MULT16_32_Q15(ratio, st.decode_mem[k]);
                    }
                }
            }

            c += 1;
            if c >= C {
                break;
            }
        }

        /* ENABLE_DEEP_PLC */
        if curr_neural {
            if let Some(ln) = lpcnet.as_deref_mut() {
                let mut buf_copy = vec![0 as f32; (C * overlap) as usize];
                let mut c = 0;
                loop {
                    let src = st.out_syn(c, N) as usize;
                    let dst = (c * overlap) as usize;
                    buf_copy[dst..dst + overlap as usize]
                        .copy_from_slice(&st.decode_mem[src..src + overlap as usize]);
                    c += 1;
                    if c >= C {
                        break;
                    }
                }

                /* Need enough samples from the PLC to cover the frame size,
                   resampling delay, and the overlap at the end. */
                let samples_needed16k = (N + SINC_ORDER + overlap) / 3;
                if !last_neural {
                    st.plc_fill = 0;
                }
                while st.plc_fill < samples_needed16k {
                    let f = st.plc_fill as usize;
                    lpcnet_plc_conceal(ln, &mut st.plc_pcm[f..]);
                    st.plc_fill += FRAME_SIZE;
                }
                /* Resample to 48 kHz. */
                for i in 0..(N + overlap) / 3 {
                    let base = (decode_buffer_size - N + 3 * i) as usize;
                    let mut sum = 0f32;
                    for j in 0..17usize {
                        sum += (3 * st.plc_pcm[i as usize + j] as i32) as f32
                            * sinc_filter[3 * j];
                    }
                    st.decode_mem[base] = sum;
                    sum = 0.;
                    for j in 0..16usize {
                        sum += (3 * st.plc_pcm[i as usize + j + 1] as i32) as f32
                            * sinc_filter[3 * j + 2];
                    }
                    st.decode_mem[base + 1] = sum;
                    sum = 0.;
                    for j in 0..16usize {
                        sum += (3 * st.plc_pcm[i as usize + j + 1] as i32) as f32
                            * sinc_filter[3 * j + 1];
                    }
                    st.decode_mem[base + 2] = sum;
                }
                let n3 = (N / 3) as usize;
                let fill = st.plc_fill as usize;
                st.plc_pcm.copy_within(n3..fill, 0);
                st.plc_fill -= N / 3;
                for i in 0..N as usize {
                    let k = (decode_buffer_size - N) as usize + i;
                    let tmp = st.decode_mem[k];
                    st.decode_mem[k] -= PREEMPHASIS * st.plc_preemphasis_mem;
                    st.plc_preemphasis_mem = tmp;
                }
                let mut overlap_mem = st.plc_preemphasis_mem;
                for i in 0..overlap as usize {
                    let k = decode_buffer_size as usize + i;
                    let tmp = st.decode_mem[k];
                    st.decode_mem[k] -= PREEMPHASIS * overlap_mem;
                    overlap_mem = tmp;
                }
                /* For now, we just do mono PLC. */
                if C == 2 {
                    let total = (decode_buffer_size + overlap) as usize;
                    let p0 = st.plane(0) as usize;
                    let p1 = st.plane(1) as usize;
                    st.decode_mem.copy_within(p0..p0 + total, p1);
                }
                let mut c = 0;
                loop {
                    /* Cross-fade with 48-kHz non-neural PLC for the first
                       2.5 ms to avoid a discontinuity. */
                    if !last_neural {
                        let base = st.out_syn(c, N) as usize;
                        for i in 0..overlap as usize {
                            st.decode_mem[base + i] = (1.0f32 - window[i])
                                * buf_copy[(c * overlap) as usize + i]
                                + window[i] * st.decode_mem[base + i];
                        }
                    }
                    c += 1;
                    if c >= C {
                        break;
                    }
                }
            }
        }
        st.prefilter_and_fold = 1;
    }

    /* Saturate to something large to avoid wrap-around. */
    st.loss_duration = IMIN(10000, loss_duration + (1 << LM));
    st.plc_duration = IMIN(10000, st.plc_duration + (1 << LM));
    /* ENABLE_DRED */
    if curr_frame_type == FRAME_DRED {
        st.plc_duration = 0;
        st.skip_plc = 0;
    }
    st.last_frame_type = curr_frame_type;
}

/* ------------------------------------------------------------------------- *
 * celt_decode_with_ec_dred() / celt_decode_with_ec() (celt_decoder.c:1100)
 * ------------------------------------------------------------------------- */

/// `celt_decode_with_ec_dred()` (`celt_decoder.c:1100`).
pub fn celt_decode_with_ec_dred(
    st: &mut CELTDecoder,
    data: Option<&[u8]>,
    len_in: i32,
    pcm: &mut [opus_res],
    frame_size_in: i32,
    dec: Option<&mut ec_ctx>,
    accum: i32,
    mut lpcnet: Option<&mut LPCNetPLCState>,
) -> i32 {
    validate_celt_decoder(st);
    let mode_Fs = st.mode.Fs;
    let mode_shortMdctSize = st.mode.shortMdctSize;
    let mode_maxLM = st.mode.maxLM;
    let mode_effEBands = st.mode.effEBands;
    let nbEBands: i32 = st.mode.nbEBands;
    let overlap: i32 = st.mode.overlap;
    let eBands: Vec<opus_int16> = st.mode.eBands.clone();
    let start: i32 = st.start;
    let mut end: i32 = st.end;
    let mut frame_size: i32 = frame_size_in * st.downsample;

    let CC: i32 = st.channels;
    let mut C: i32 = st.stream_channels;

    /* `data_off` replaces the C `data++` for the TOC/padding parsing. */
    let mut data_off: usize = 0;
    let mut len: i32 = len_in;
    let mut LM: i32 = 0;

    if st.signalling != 0 && data.is_some() {
        let d = data.unwrap();
        let mut data0 = d.get(0).copied().unwrap_or(0) as i32;
        /* CUSTOM_MODES: convert "standard mode" to the Opus header. */
        if mode_Fs == 48000 && mode_shortMdctSize == 120 {
            data0 = fromOpus(data0 as u8);
            if data0 < 0 {
                return ffi::OPUS_INVALID_PACKET;
            }
        }
        st.end = IMAX(1, mode_effEBands - 2 * (data0 >> 5));
        end = st.end;
        LM = (data0 >> 3) & 0x3;
        C = 1 + ((data0 >> 2) & 0x1);
        if (d[0] & 0x03) == 0x03 {
            data_off += 1;
            len -= 1;
            if len <= 0 {
                return ffi::OPUS_INVALID_PACKET;
            }
            if (d[data_off] & 0x40) != 0 {
                let mut padding = 0;
                data_off += 1;
                len -= 1;
                loop {
                    if len <= 0 {
                        return ffi::OPUS_INVALID_PACKET;
                    }
                    let p = d[data_off] as i32;
                    data_off += 1;
                    len -= 1;
                    let tmp = if p == 255 { 254 } else { p };
                    len -= tmp;
                    padding += tmp;
                    if p != 255 {
                        break;
                    }
                }
                padding -= 1;
                if len <= 0 || padding < 0 {
                    return ffi::OPUS_INVALID_PACKET;
                }
            }
        } else {
            data_off += 1;
            len -= 1;
        }
        if LM > mode_maxLM {
            return ffi::OPUS_INVALID_PACKET;
        }
        if frame_size < mode_shortMdctSize << LM {
            return ffi::OPUS_BUFFER_TOO_SMALL;
        } else {
            frame_size = mode_shortMdctSize << LM;
        }
    } else {
        LM = 0;
        while LM <= mode_maxLM && (mode_shortMdctSize << LM) != frame_size {
            LM += 1;
        }
        if LM > mode_maxLM {
            return ffi::OPUS_BAD_ARG;
        }
    }
    let M: i32 = 1 << LM;

    if len < 0 || len > 1275 {
        return ffi::OPUS_BAD_ARG;
    }

    let N: i32 = M * mode_shortMdctSize;
    let decode_buffer_size = DECODE_BUFFER_SIZE;

    let mut effEnd = end;
    if effEnd > mode_effEBands {
        effEnd = mode_effEBands;
    }

    if data.is_none() || len <= 1 {
        celt_decode_lost(st, N, LM, lpcnet.as_deref_mut());
        let p0 = st.out_syn(0, N) as usize;
        let p1 = if CC == 2 { st.out_syn(1, N) as usize } else { p0 };
        let in0 = &st.decode_mem[p0..];
        let in1 = &st.decode_mem[p1..];
        let inp: [&[celt_sig]; 2] = [in0, in1];
        let downsample = st.downsample;
        deemphasis(
            &inp[..CC as usize],
            pcm,
            N,
            CC,
            downsample,
            &st.mode.preemph,
            &mut st.preemph_memD,
            accum,
        );
        return frame_size / st.downsample;
    }
    /* ENABLE_DEEP_PLC */
    if let Some(l) = lpcnet.as_deref_mut() {
        /* Make sure opus_decode_native() knows we're no longer in PLC. */
        l.blend = 0;
    }

    /* Check if there are at least two packets received consecutively before
     * turning on the pitch-based PLC */
    if st.loss_duration == 0 {
        st.skip_plc = 0;
    }

    let mut local_dec = ec_dec_init(&[]);
    let dec_ref: &mut ec_ctx = match dec {
        Some(d) => d,
        None => {
            let d = data.unwrap();
            local_dec = ec_dec_init(&d[data_off..(data_off + len as usize)]);
            &mut local_dec
        }
    };
    let dec: &mut ec_ctx = dec_ref;

    if C == 1 {
        for i in 0..nbEBands {
            st.oldEBands[i as usize] =
                MAXG(st.oldEBands[i as usize], st.oldEBands[(nbEBands + i) as usize]);
        }
    }

    let mut total_bits: opus_int32 = len * 8;
    let mut tell: opus_int32 = dec.ec_tell();

    let mut silence: i32;
    if tell >= total_bits {
        silence = 1;
    } else if tell == 1 {
        silence = ec_dec_bit_logp(dec, 15);
    } else {
        silence = 0;
    }
    if silence != 0 {
        /* Pretend we've read all the remaining bits */
        tell = len * 8;
        dec.nbits_total += tell - dec.ec_tell();
    }

    let mut postfilter_gain: opus_val16 = 0.;
    let mut postfilter_pitch: i32 = 0;
    let mut postfilter_tapset: i32 = 0;
    if start == 0 && tell + 16 <= total_bits {
        if ec_dec_bit_logp(dec, 1) != 0 {
            let octave = ec_dec_uint(dec, 6) as i32;
            postfilter_pitch =
                (16 << octave) + ec_dec_bits(dec, (4 + octave) as u32) as i32 - 1;
            let qg = ec_dec_bits(dec, 3) as i32;
            if dec.ec_tell() + 2 <= total_bits {
                postfilter_tapset = ec_dec_icdf(dec, &tapset_icdf, 2);
            }
            postfilter_gain = QCONST16(0.09375f32, 15) * (qg + 1) as f32;
        }
        tell = dec.ec_tell();
    }

    let mut isTransient: i32;
    if LM > 0 && tell + 3 <= total_bits {
        isTransient = ec_dec_bit_logp(dec, 3);
        tell = dec.ec_tell();
    } else {
        isTransient = 0;
    }

    let shortBlocks: i32 = if isTransient != 0 { M } else { 0 };

    /* Decode the global flags (first symbols in the stream). */
    let intra_ener = if tell + 3 <= total_bits {
        ec_dec_bit_logp(dec, 3)
    } else {
        0
    };
    /* If recovering from packet loss, make sure we make the energy prediction
       safe to reduce the risk of getting loud artifacts. */
    if intra_ener == 0 && st.loss_duration != 0 {
        let mut c = 0;
        loop {
            let mut safety: celt_glog = 0.;
            let missing = IMIN(10, st.loss_duration >> LM);
            if LM == 0 {
                safety = GCONST(1.5f32);
            } else if LM == 1 {
                safety = GCONST(0.5f32);
            }
            for i in start..end {
                let idx = (c * nbEBands + i) as usize;
                if st.oldEBands[idx] < MAXG(st.oldLogE[idx], st.oldLogE2[idx]) {
                    /* If energy is going down already, continue the trend. */
                    let E0: opus_val32 = st.oldEBands[idx];
                    let E1: opus_val32 = st.oldLogE[idx];
                    let E2: opus_val32 = st.oldLogE2[idx];
                    let mut slope: opus_val32 = MAX32(E1 - E0, HALF32(E2 - E0));
                    slope = MING(slope, GCONST(2.0f32));
                    let E0 = E0 - MAX32(0., (1 + missing) as opus_val32 * slope);
                    st.oldEBands[idx] = MAX32(-GCONST(20.0f32), E0);
                } else {
                    /* Otherwise take the min of the last frames. */
                    st.oldEBands[idx] = MING(
                        MING(st.oldEBands[idx], st.oldLogE[idx]),
                        st.oldLogE2[idx],
                    );
                }
                /* Shorter frames have more natural fluctuations -- play it safe. */
                st.oldEBands[idx] -= safety;
            }
            c += 1;
            if c >= 2 {
                break;
            }
        }
    }

    /* Get band energies */
    unquant_coarse_energy(&st.mode, start, end, &mut st.oldEBands, intra_ener, dec, C, LM);

    let mut tf_res = vec![0; nbEBands as usize];
    tf_decode(start, end, isTransient, &mut tf_res, LM, dec);

    tell = dec.ec_tell();
    let mut spread_decision = SPREAD_NORMAL;
    if tell + 4 <= total_bits {
        spread_decision = ec_dec_icdf(dec, &spread_icdf, 5);
    }

    let mut cap = vec![0; nbEBands as usize];
    init_caps(&st.mode, &mut cap, LM, C);

    let mut offsets = vec![0; nbEBands as usize];

    let mut dynalloc_logp = 6;
    total_bits <<= BITRES;
    tell = ec_tell_frac(dec) as opus_int32;
    for i in start..end {
        let width: i32;
        let quanta: i32;
        let mut dynalloc_loop_logp: i32;
        let mut boost: i32;
        width = C * ((eBands[(i + 1) as usize] as i32) - (eBands[i as usize] as i32)) << LM;
        /* quanta is 6 bits, but no more than 1 bit/sample
           and no less than 1/8 bit/sample */
        quanta = IMIN(width << BITRES, IMAX(6 << BITRES, width));
        dynalloc_loop_logp = dynalloc_logp;
        boost = 0;
        while tell.wrapping_add(dynalloc_loop_logp << BITRES) < total_bits
            && boost < cap[i as usize]
        {
            let flag = ec_dec_bit_logp(dec, dynalloc_loop_logp as u32);
            tell = ec_tell_frac(dec) as opus_int32;
            if flag == 0 {
                break;
            }
            boost += quanta;
            total_bits -= quanta;
            dynalloc_loop_logp = 1;
        }
        offsets[i as usize] = boost;
        /* Making dynalloc more likely */
        if boost > 0 {
            dynalloc_logp = IMAX(2, dynalloc_logp - 1);
        }
    }

    let mut fine_quant = vec![0; nbEBands as usize];
    let alloc_trim = if tell.wrapping_add(6 << BITRES) <= total_bits {
        ec_dec_icdf(dec, &trim_icdf, 7)
    } else {
        5
    };

    let mut bits: opus_int32 = ((len * 8) << BITRES) - ec_tell_frac(dec) as opus_int32 - 1;
    let anti_collapse_rsv: i32 = if isTransient != 0 && LM >= 2 && bits >= ((LM + 2) << BITRES) {
        1 << BITRES
    } else {
        0
    };
    bits -= anti_collapse_rsv;

    let mut pulses = vec![0; nbEBands as usize];
    let mut fine_priority = vec![0; nbEBands as usize];
    let mut intensity: i32 = 0;
    let mut dual_stereo: i32 = 0;
    let mut balance: opus_int32 = 0;

    let codedBands = clt_compute_allocation(
        &st.mode,
        start,
        end,
        &offsets,
        &cap,
        alloc_trim,
        &mut intensity,
        &mut dual_stereo,
        bits,
        &mut balance,
        &mut pulses,
        &mut fine_quant,
        &mut fine_priority,
        C,
        LM,
        dec,
        0,
        0,
        0,
    );

    unquant_fine_energy(
        &st.mode,
        start,
        end,
        &mut st.oldEBands,
        None,
        &fine_quant,
        dec,
        C,
    );

    let mut X = vec![0 as celt_norm; (C * N) as usize];

    {
        let mut c = 0;
        loop {
            let plane = st.plane(c) as usize;
            let n = N as usize;
            let total = (decode_buffer_size + overlap) as usize;
            st.decode_mem[plane..plane + total].copy_within(n..total, 0);
            c += 1;
            if c >= CC {
                break;
            }
        }
    }

    /* Decode fixed codebook */
    let mut collapse_masks = vec![0u8; (C * nbEBands) as usize];

    {
        let (x0, x1) = X.split_at_mut(N as usize);
        let y = if C == 2 { Some(x1) } else { None };
        quant_all_bands(
            0,
            &st.mode,
            start,
            end,
            x0,
            y,
            &mut collapse_masks,
            &[],
            &pulses,
            shortBlocks,
            spread_decision,
            dual_stereo,
            intensity,
            &tf_res,
            len * (8 << BITRES) - anti_collapse_rsv,
            balance,
            dec,
            LM,
            codedBands,
            &mut st.rng,
            0,
            st.arch,
            st.disable_inv,
        );
    }

    let anti_collapse_on: i32 = if anti_collapse_rsv > 0 {
        ec_dec_bits(dec, 1) as i32
    } else {
        0
    };
    unquant_energy_finalise(
        &st.mode,
        start,
        end,
        Some(&mut st.oldEBands),
        &fine_quant,
        &fine_priority,
        len * 8 - dec.ec_tell(),
        dec,
        C,
    );
    if anti_collapse_on != 0 {
        anti_collapse(
            &st.mode,
            &mut X,
            &collapse_masks,
            LM,
            C,
            N,
            start,
            end,
            &st.oldEBands,
            &st.oldLogE,
            &st.oldLogE2,
            &pulses,
            st.rng,
            0,
            st.arch,
        );
    }

    if silence != 0 {
        for i in 0..(C * nbEBands) as usize {
            st.oldEBands[i] = -GCONST(28.0f32);
        }
    }
    if st.prefilter_and_fold != 0 {
        prefilter_and_fold(st, N);
    }
    let out_off = [st.out_syn(0, N), st.out_syn(1, N)];
    celt_synthesis(
        &st.mode,
        &mut X,
        &mut st.decode_mem,
        out_off,
        &st.oldEBands,
        start,
        effEnd,
        C,
        CC,
        isTransient,
        LM,
        st.downsample,
        silence,
        st.arch,
    );

    {
        let mut c = 0;
        loop {
            st.postfilter_period = IMAX(st.postfilter_period, COMBFILTER_MINPERIOD);
            st.postfilter_period_old = IMAX(st.postfilter_period_old, COMBFILTER_MINPERIOD);
            let plane = st.plane(c) as usize;
            let plane_end = plane + (decode_buffer_size + overlap) as usize;
            let x_off = decode_buffer_size - N;
            let shortMdctSize = st.mode.shortMdctSize;
            comb_filter_in_place(
                &mut st.decode_mem[plane..plane_end],
                x_off,
                st.postfilter_period_old,
                st.postfilter_period,
                shortMdctSize,
                st.postfilter_gain_old,
                st.postfilter_gain,
                st.postfilter_tapset_old,
                st.postfilter_tapset,
                &st.mode.window,
                overlap,
                st.arch,
            );
            if LM != 0 {
                comb_filter_in_place(
                    &mut st.decode_mem[plane..plane_end],
                    x_off + shortMdctSize,
                    st.postfilter_period,
                    postfilter_pitch,
                    N - shortMdctSize,
                    st.postfilter_gain,
                    postfilter_gain,
                    st.postfilter_tapset,
                    postfilter_tapset,
                    &st.mode.window,
                    overlap,
                    st.arch,
                );
            }
            c += 1;
            if c >= CC {
                break;
            }
        }
    }
    st.postfilter_period_old = st.postfilter_period;
    st.postfilter_gain_old = st.postfilter_gain;
    st.postfilter_tapset_old = st.postfilter_tapset;
    st.postfilter_period = postfilter_pitch;
    st.postfilter_gain = postfilter_gain;
    st.postfilter_tapset = postfilter_tapset;
    if LM != 0 {
        st.postfilter_period_old = st.postfilter_period;
        st.postfilter_gain_old = st.postfilter_gain;
        st.postfilter_tapset_old = st.postfilter_tapset;
    }

    if C == 1 {
        /* `OPUS_COPY(&oldBandE[nbEBands], oldBandE, nbEBands)` */
        let nbe = nbEBands as usize;
        let (lo, hi) = st.oldEBands.split_at_mut(nbe);
        hi[..nbe].copy_from_slice(&lo[..nbe]);
    }

    if isTransient == 0 {
        st.oldLogE2.copy_from_slice(&st.oldLogE);
        st.oldLogE.copy_from_slice(&st.oldEBands);
    } else {
        for i in 0..(2 * nbEBands) as usize {
            st.oldLogE[i] = MING(st.oldLogE[i], st.oldEBands[i]);
        }
    }
    /* In normal circumstances, we only allow the noise floor to increase by
       up to 2.4 dB/second, but when we're in DTX we give the weight of
       all missing packets to the update packet. */
    let max_background_increase: celt_glog =
        (IMIN(160, st.loss_duration + M) as opus_val32) * GCONST(0.001f32);
    for i in 0..(2 * nbEBands) as usize {
        st.backgroundLogE[i] =
            MING(st.backgroundLogE[i] + max_background_increase, st.oldEBands[i]);
    }
    /* In case start or end were to change */
    let mut c = 0;
    loop {
        for i in 0..start {
            let idx = (c * nbEBands + i) as usize;
            st.oldEBands[idx] = 0.;
            st.oldLogE[idx] = -GCONST(28.0f32);
            st.oldLogE2[idx] = -GCONST(28.0f32);
        }
        for i in end..nbEBands {
            let idx = (c * nbEBands + i) as usize;
            st.oldEBands[idx] = 0.;
            st.oldLogE[idx] = -GCONST(28.0f32);
            st.oldLogE2[idx] = -GCONST(28.0f32);
        }
        c += 1;
        if c >= 2 {
            break;
        }
    }
    st.rng = dec.rng;

    {
        let p0 = st.out_syn(0, N) as usize;
        let p1 = if CC == 2 { st.out_syn(1, N) as usize } else { p0 };
        let in0 = &st.decode_mem[p0..];
        let in1 = &st.decode_mem[p1..];
        let inp: [&[celt_sig]; 2] = [in0, in1];
        let downsample = st.downsample;
        deemphasis(
            &inp[..CC as usize],
            pcm,
            N,
            CC,
            downsample,
            &st.mode.preemph,
            &mut st.preemph_memD,
            accum,
        );
    }
    st.loss_duration = 0;
    st.plc_duration = 0;
    st.last_frame_type = FRAME_NORMAL;
    st.prefilter_and_fold = 0;

    if dec.ec_tell() > 8 * len {
        return ffi::OPUS_INTERNAL_ERROR;
    }
    if dec.ec_get_error() != 0 {
        st.error = 1;
    }
    frame_size / st.downsample
}

/// `celt_decode_with_ec()` (`celt_decoder.c:1615`).
pub fn celt_decode_with_ec(
    st: &mut CELTDecoder,
    data: Option<&[u8]>,
    len: i32,
    pcm: &mut [opus_res],
    frame_size: i32,
    dec: Option<&mut ec_ctx>,
    accum: i32,
) -> i32 {
    celt_decode_with_ec_dred(st, data, len, pcm, frame_size, dec, accum, None)
}

/* ------------------------------------------------------------------------- *
 * CUSTOM_MODES entry points (celt_decoder.c:1626)
 * ------------------------------------------------------------------------- */

/// `opus_custom_decode()` (`celt_decoder.c:1634`, float body).
pub fn opus_custom_decode(
    st: &mut CELTDecoder,
    data: Option<&[u8]>,
    len: i32,
    pcm: &mut [opus_int16],
    frame_size: i32,
) -> i32 {
    let C = st.channels;
    let N = frame_size;

    let mut out = vec![0 as opus_res; (C * N) as usize];
    let ret = celt_decode_with_ec(st, data, len, &mut out, frame_size, None, 0);
    if ret > 0 {
        for j in 0..(C * ret) as usize {
            pcm[j] = RES2INT16(out[j]);
        }
    }
    ret
}

/// `opus_custom_decode24()` (`celt_decoder.c:1663`, float body).
pub fn opus_custom_decode24(
    st: &mut CELTDecoder,
    data: Option<&[u8]>,
    len: i32,
    pcm: &mut [opus_int32],
    frame_size: i32,
) -> i32 {
    let C = st.channels;
    let N = frame_size;

    let mut out = vec![0 as opus_res; (C * N) as usize];
    let ret = celt_decode_with_ec(st, data, len, &mut out, frame_size, None, 0);
    if ret > 0 {
        for j in 0..(C * ret) as usize {
            pcm[j] = RES2INT24(out[j]);
        }
    }
    ret
}

/// `opus_custom_decode_float()` (`celt_decoder.c:1690`, float body).
pub fn opus_custom_decode_float(
    st: &mut CELTDecoder,
    data: Option<&[u8]>,
    len: i32,
    pcm: &mut [f32],
    frame_size: i32,
) -> i32 {
    celt_decode_with_ec(st, data, len, pcm, frame_size, None, 0)
}

/* ------------------------------------------------------------------------- *
 * Unit tests for the pure helpers
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::super::celt::{
        CELT_SET_CHANNELS_REQUEST, CELT_SET_END_BAND_REQUEST, CELT_SET_START_BAND_REQUEST,
        CELT_GET_AND_CLEAR_ERROR_REQUEST,
    };
    use super::*;

    #[test]
    fn get_size_matches_the_c_formula() {
        let mut error = 0;
        let mode = opus_custom_mode_create(48000, 960, &mut error).unwrap();
        /* sizeof(struct CELTDecoder) + (channels*(2048+120)-1)*4
           + 4*2*21*4 + channels*24*4 with sizeof(struct)=1408. */
        assert_eq!(opus_custom_decoder_get_size(&mode, 1), 10844);
        assert_eq!(opus_custom_decoder_get_size(&mode, 2), 19612);
        assert_eq!(celt_decoder_get_size(1), 10844);
        assert_eq!(celt_decoder_get_size(2), 19612);
    }

    #[test]
    fn init_and_reset_state() {
        let mut st = CELTDecoder::new();
        assert_eq!(celt_decoder_init(&mut st, 48000, 2), ffi::OPUS_OK);
        assert_eq!(st.downsample, 1);
        assert_eq!(st.channels, 2);
        assert_eq!(st.stream_channels, 2);
        assert_eq!(st.start, 0);
        assert_eq!(st.end, 21);
        assert_eq!(st.signalling, 1);
        assert_eq!(st.disable_inv, 0);
        assert_eq!(st.skip_plc, 1);
        assert_eq!(st.last_frame_type, FRAME_NONE);
        assert_eq!(st.decode_mem.len(), 2 * (DECODE_BUFFER_SIZE + 120) as usize);
        assert_eq!(st.oldEBands.len(), 42);
        assert_eq!(st.plc_pcm.len(), PLC_UPDATE_SAMPLES as usize);
        assert!(st.oldLogE.iter().all(|&x| x == -28.0));
        assert!(st.oldLogE2.iter().all(|&x| x == -28.0));
        assert!(st.oldEBands.iter().all(|&x| x == 0.0));

        /* Mono disables phase inversion by default. */
        let mut st1 = CELTDecoder::new();
        assert_eq!(celt_decoder_init(&mut st1, 16000, 1), ffi::OPUS_OK);
        assert_eq!(st1.downsample, 3);
        assert_eq!(st1.disable_inv, 1);

        /* Bad rate and bad channel count. */
        let mut st2 = CELTDecoder::new();
        assert_eq!(celt_decoder_init(&mut st2, 12345, 2), ffi::OPUS_BAD_ARG);
        let mut st3 = CELTDecoder::new();
        let mut e = 0;
        let mode = opus_custom_mode_create(48000, 960, &mut e).unwrap();
        assert_eq!(opus_custom_decoder_init(&mut st3, mode, 3), ffi::OPUS_BAD_ARG);
    }

    #[test]
    fn ctl_scalar_requests() {
        let mut st = CELTDecoder::new();
        celt_decoder_init(&mut st, 48000, 2);

        let mut v = 7;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(st.complexity, 7);
        let mut out = 0;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, ffi::OPUS_GET_COMPLEXITY_REQUEST, &mut out),
            ffi::OPUS_OK
        );
        assert_eq!(out, 7);
        let mut bad = 11;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut bad),
            ffi::OPUS_BAD_ARG
        );

        let mut v = 5;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, CELT_SET_START_BAND_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(st.start, 5);
        let mut bad = -1;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, CELT_SET_START_BAND_REQUEST, &mut bad),
            ffi::OPUS_BAD_ARG
        );
        let mut v = 21;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, CELT_SET_END_BAND_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(st.end, 21);
        let mut bad = 0;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, CELT_SET_END_BAND_REQUEST, &mut bad),
            ffi::OPUS_BAD_ARG
        );

        let mut v = 1;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, CELT_SET_CHANNELS_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(st.stream_channels, 1);

        st.error = 1;
        let mut e = 0;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, CELT_GET_AND_CLEAR_ERROR_REQUEST, &mut e),
            ffi::OPUS_OK
        );
        assert_eq!(e, 1);
        assert_eq!(st.error, 0);

        let mut v = 0;
        assert_eq!(
            opus_custom_decoder_ctl(&mut st, ffi::OPUS_GET_LOOKAHEAD_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        assert_eq!(v, 120);

        assert_eq!(
            opus_custom_decoder_ctl(&mut st, ffi::OPUS_GET_FINAL_RANGE_REQUEST, &mut v),
            ffi::OPUS_OK
        );
        let mut v = 1;
        assert_eq!(
            opus_custom_decoder_ctl(
                &mut st,
                ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST,
                &mut v
            ),
            ffi::OPUS_OK
        );
        assert_eq!(st.disable_inv, 1);
        let mut v = 0;
        assert_eq!(
            opus_custom_decoder_ctl(
                &mut st,
                ffi::OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST,
                &mut v
            ),
            ffi::OPUS_OK
        );
        assert_eq!(v, 1);

        assert_eq!(
            opus_custom_decoder_ctl(&mut st, 999999, &mut v),
            ffi::OPUS_UNIMPLEMENTED
        );
    }

    #[test]
    fn synthesises_silence_to_zero() {
        let mut error = 0;
        let mode = opus_custom_mode_create(48000, 960, &mut error).unwrap();
        let mut st = CELTDecoder::new();
        opus_custom_decoder_init(&mut st, mode, 1);
        let N = 120;
        let mut X = vec![0 as celt_norm; N as usize];
        let out_off = [st.out_syn(0, N), st.out_syn(1, N)];
        celt_synthesis(
            &st.mode,
            &mut X,
            &mut st.decode_mem,
            out_off,
            &st.oldEBands,
            0,
            21,
            1,
            1,
            0,
            0,
            1,
            1,
            ARCH,
        );
        /* With silence=1 `denormalise_bands` zeroes the spectrum, so the
           whole IMDCT output is zero. */
        let base = out_off[0] as usize;
        assert!(st.decode_mem[base..base + N as usize].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn tf_decode_entries_are_in_range() {
        let mut dec = ec_dec_init(&[0u8; 4]);
        let mut tf_res = vec![0i32; 21];
        tf_decode(0, 21, 0, &mut tf_res, 0, &mut dec);
        /* LM == 0: the per-band flags are actually read and the select table
           is applied, so every entry must be a valid table value. */
        assert!(tf_res.iter().all(|&v| (-3..=3).contains(&v)));
    }

    #[test]
    fn plc_noise_frame_is_deterministic() {
        let mut st = CELTDecoder::new();
        celt_decoder_init(&mut st, 48000, 1);
        let mut pcm = vec![0f32; 480];
        /* Two consecutive losses: the first takes the noise branch (skip_plc
           is set on reset), the second is still noise (plc_duration >= 40). */
        let r1 = celt_decode_with_ec(&mut st, None, 0, &mut pcm, 480, None, 0);
        assert_eq!(r1, 480);
        let r2 = celt_decode_with_ec(&mut st, None, 0, &mut pcm, 480, None, 0);
        assert_eq!(r2, 480);
        /* `loss_duration += 1<<LM` per frame, LM = 2 here. */
        assert_eq!(st.loss_duration, 2 * (1 << 2));
        assert_eq!(st.last_frame_type, FRAME_PLC_NOISE);
        /* A reset clears the loss history. */
        let mut v = 0;
        opus_custom_decoder_ctl(&mut st, ffi::OPUS_RESET_STATE, &mut v);
        assert_eq!(st.loss_duration, 0);
        assert_eq!(st.rng, 0);
    }
}

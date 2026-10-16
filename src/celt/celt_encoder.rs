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

//! Transliteration of `celt/celt_encoder.c` (libopus 1.6.1) into safe Rust.
//!
//! This is a derived work of the BSD-3-Clause libopus sources; the copyright
//! notice above is the one of the original file.
//!
//! Configuration (docs/porting-conventions.md): floating point, `CUSTOM_MODES`
//! and `ENABLE_HARDENING` defined; `FIXED_POINT`, `FLOAT_APPROX`,
//! `ENABLE_QEXT`, `RESYNTH` and `FUZZING` undefined.  Consequently:
//!
//! * every `#ifdef ENABLE_QEXT` block is dropped (no 96 kHz mode, no extra
//!   QEXT payload, `QEXT_SCALE(x) == x`), and `#ifdef RESYNTH` block is
//!   dropped, so the encoder never calls `celt_synthesis`/`deemphasis`/
//!   `anti_collapse` and the `prefilter_*_old` fields and `syn_mem` do not
//!   exist in the compiled C struct (confirmed with `nm celt/celt_encoder.o`:
//!   no `celt_synthesis`, `deemphasis` or `anti_collapse` undefined symbol);
//! * `FIXED_POINT` is undefined, so every `SHR32`/`SHL32`/`PSHR32`/`SROUND16`
//!   is the identity and `MULT16_*` is a plain `f32` multiply; the macros are
//!   still written out so the code can be diffed against the C.
//!
//! The C encoder state is a single allocation whose tail holds the
//! `in_mem` / `prefilter_mem` / `oldBandE` / `oldLogE` / `oldLogE2` /
//! `energyError` arrays, reached through pointer arithmetic.  The Rust port
//! keeps them as separate owned `Vec`s with the same contents, and
//! [`opus_custom_encoder_get_size`] still returns the **C** byte count (the
//! `sizeof(struct CELTEncoder)` of this build is 256, verified against the
//! reference archive: `celt_encoder_get_size(1) == 5164`).
//!
//! `opus_custom_encoder_ctl()` is variadic in C and takes `int`, `int*`,
//! `AnalysisInfo*`, `SILKInfo*`, `celt_glog*` and `const CELTMode**`
//! arguments.  Rust has no varargs, so the scalar requests keep the C name and
//! signature with a single `&mut i32` value, and the pointer requests get the
//! `opus_custom_encoder_ctl_*` entry points right below it.

use super::arch::*;
use super::bands::{
    compute_band_energies, haar1, hysteresis_decision, normalise_bands, quant_all_bands,
    spreading_decision,
};
use super::celt::{
    comb_filter, init_caps, resampling_factor, spread_icdf, tapset_icdf, tf_select_table, toOpus,
    trim_icdf, AnalysisInfo, SILKInfo, COMBFILTER_MAXPERIOD, COMBFILTER_MINPERIOD,
};
use super::entcode::{ec_ctx, ec_tell_frac, BITRES, EC_ILOG};
use super::entenc::{
    ec_enc_bit_logp, ec_enc_bits, ec_enc_done, ec_enc_icdf, ec_enc_init, ec_enc_shrink, ec_enc_uint,
};
use super::kernels::celt_inner_prod;
use super::arch::celt_isnan;
use super::celt::bitrate_to_bits;
use super::mathops::{
    celt_exp2_db, celt_log2, celt_maxabs_res, celt_rcp, celt_sqrt, frac_div32_q29, PI,
};
use super::mdct::clt_mdct_forward;
use super::modes::{opus_custom_mode_create, CELTMode};
use super::pitch::{pitch_downsample, pitch_search, remove_doubling};
use super::quant_bands::{
    amp2Log2, quant_coarse_energy, quant_energy_finalise, quant_fine_energy, DB_SHIFT,
};
use super::rate::clt_compute_allocation;
use crate::ffi::{
    OPUS_ALLOC_FAIL, OPUS_BAD_ARG, OPUS_BITRATE_MAX, OPUS_GET_FINAL_RANGE_REQUEST,
    OPUS_GET_LSB_DEPTH_REQUEST, OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST, OPUS_INTERNAL_ERROR,
    OPUS_OK, OPUS_RESET_STATE, OPUS_SET_BITRATE_REQUEST, OPUS_SET_COMPLEXITY_REQUEST,
    OPUS_SET_LSB_DEPTH_REQUEST, OPUS_SET_PACKET_LOSS_PERC_REQUEST,
    OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST, OPUS_SET_VBR_CONSTRAINT_REQUEST,
    OPUS_SET_VBR_REQUEST, OPUS_UNIMPLEMENTED,
};
use crate::types::*;

/// `opus_select_arch()` returns 4 on the host the reference configuration is
/// pinned to (docs/porting-conventions.md §1.4).  The encoder stores it and
/// only ever passes it to the DSP kernels, whose SSE/AVX2 emulations in
/// `crate::celt::kernels` do not depend on it.
pub const OPUS_ARCH: i32 = 4;

/// `sizeof(struct CELTEncoder)` of the reference build (float, no `RESYNTH`,
/// no `ENABLE_QEXT`).  Verified against `reference/lib-ref/libopus.a`:
/// `celt_encoder_get_size(1) == 5164`.
pub const SIZEOF_CELT_ENCODER: i32 = 256;

/// `LEAK_BANDS` is re-exported from `celt.rs`.
use super::celt::LEAK_BANDS;

/* ------------------------------------------------------------------------- *
 * small float-build helpers matching the C macros that only exist because
 * `celt_glog == opus_val32 == float`.
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

/// `OPUS_COPY(dst, src, n)` for `f32` slices.
#[inline(always)]
fn opus_copy_f32(dst: &mut [f32], src: &[f32]) {
    dst.copy_from_slice(src);
}

/* ------------------------------------------------------------------------- *
 * struct OpusCustomEncoder (`celt/celt_encoder.c:63`)
 * ------------------------------------------------------------------------- */

/// Encoder state (`struct OpusCustomEncoder`, `CELTEncoder`).
///
/// The trailing arrays of the C allocation are separate `Vec`s here (see the
/// module docs).  The `RESYNTH` fields (`prefilter_*_old`, `syn_mem`) are not
/// part of this build.
pub struct OpusCustomEncoder {
    pub mode: Box<CELTMode>,
    pub channels: i32,
    pub stream_channels: i32,

    pub force_intra: i32,
    pub clip: i32,
    pub disable_pf: i32,
    pub complexity: i32,
    pub upsample: i32,
    pub start: i32,
    pub end: i32,

    pub bitrate: opus_int32,
    pub vbr: i32,
    pub signalling: i32,
    pub constrained_vbr: i32,
    pub loss_rate: i32,
    pub lsb_depth: i32,
    pub lfe: i32,
    pub disable_inv: i32,
    pub arch: i32,

    /* Everything beyond this point gets cleared on a reset. */
    pub rng: opus_uint32,
    pub spread_decision: i32,
    pub delayedIntra: opus_val32,
    pub tonal_average: i32,
    pub lastCodedBands: i32,
    pub hf_average: i32,
    pub tapset_decision: i32,

    pub prefilter_period: i32,
    pub prefilter_gain: opus_val16,
    pub prefilter_tapset: i32,
    pub consec_transient: i32,
    pub analysis: AnalysisInfo,
    pub silk_info: SILKInfo,

    pub preemph_memE: [opus_val32; 2],
    pub preemph_memD: [opus_val32; 2],

    /* VBR-related parameters */
    pub vbr_reservoir: opus_int32,
    pub vbr_drift: opus_int32,
    pub vbr_offset: opus_int32,
    pub vbr_count: opus_int32,
    pub overlap_max: opus_val32,
    pub stereo_saving: opus_val16,
    pub intensity: i32,
    /// C: `celt_glog *energy_mask` (borrowed caller memory).  The Rust port
    /// copies the values handed to `OPUS_SET_ENERGY_MASK` and stores `None`
    /// for the C `NULL`; the encoder only ever reads them.
    pub energy_mask: Option<Vec<celt_glog>>,
    pub spec_avg: celt_glog,

    /* celt_sig in_mem[channels*mode->overlap] */
    pub in_mem: Vec<celt_sig>,
    /* celt_sig prefilter_mem[channels*COMBFILTER_MAXPERIOD] */
    pub prefilter_mem: Vec<celt_sig>,
    /* celt_glog oldBandE[channels*mode->nbEBands] */
    pub oldBandE: Vec<celt_glog>,
    /* celt_glog oldLogE[channels*mode->nbEBands] */
    pub oldLogE: Vec<celt_glog>,
    /* celt_glog oldLogE2[channels*mode->nbEBands] */
    pub oldLogE2: Vec<celt_glog>,
    /* celt_glog energyError[channels*mode->nbEBands] */
    pub energyError: Vec<celt_glog>,
}

impl OpusCustomEncoder {
    /// An all-zero state with a placeholder mode.  The C caller allocates
    /// `opus_custom_encoder_get_size()` bytes and then calls
    /// `opus_custom_encoder_init*()`, which clears the whole allocation; the
    /// Rust twin mirrors that with [`OpusCustomEncoder::zeroed`] followed by
    /// [`opus_custom_encoder_init_arch`].
    pub fn zeroed() -> Self {
        OpusCustomEncoder {
            mode: Box::new(CELTMode::default()),
            channels: 0,
            stream_channels: 0,
            force_intra: 0,
            clip: 0,
            disable_pf: 0,
            complexity: 0,
            upsample: 0,
            start: 0,
            end: 0,
            bitrate: 0,
            vbr: 0,
            signalling: 0,
            constrained_vbr: 0,
            loss_rate: 0,
            lsb_depth: 0,
            lfe: 0,
            disable_inv: 0,
            arch: 0,
            rng: 0,
            spread_decision: 0,
            delayedIntra: 0.0,
            tonal_average: 0,
            lastCodedBands: 0,
            hf_average: 0,
            tapset_decision: 0,
            prefilter_period: 0,
            prefilter_gain: 0.0,
            prefilter_tapset: 0,
            consec_transient: 0,
            analysis: AnalysisInfo::default(),
            silk_info: SILKInfo::default(),
            preemph_memE: [0.0; 2],
            preemph_memD: [0.0; 2],
            vbr_reservoir: 0,
            vbr_drift: 0,
            vbr_offset: 0,
            vbr_count: 0,
            overlap_max: 0.0,
            stereo_saving: 0.0,
            intensity: 0,
            energy_mask: None,
            spec_avg: 0.0,
            in_mem: Vec::new(),
            prefilter_mem: Vec::new(),
            oldBandE: Vec::new(),
            oldLogE: Vec::new(),
            oldLogE2: Vec::new(),
            energyError: Vec::new(),
        }
    }

    /// `st->mode`
    #[inline(always)]
    pub fn mode(&self) -> &CELTMode {
        &self.mode
    }

    /// `st->channels*mode->overlap`: the length of the C `in_mem` array.
    #[inline(always)]
    pub fn in_mem_len(&self) -> usize {
        (self.channels * self.mode.overlap) as usize
    }
}

/* ------------------------------------------------------------------------- *
 * celt_encoder_get_size / opus_custom_encoder_get_size
 * ------------------------------------------------------------------------- */

/// `celt_encoder_get_size()` (`celt/celt_encoder.c:144`).
pub fn celt_encoder_get_size(channels: i32) -> i32 {
    /* `#ifdef ENABLE_QEXT` is undefined, so the 48 kHz mode is used. */
    let mut err = 0i32;
    let mode = match opus_custom_mode_create(48000, 960, &mut err) {
        Some(m) => m,
        None => return 0,
    };
    opus_custom_encoder_get_size(&mode, channels)
}

/// `opus_custom_encoder_get_size()` (`celt/celt_encoder.c:154`).
///
/// Returns the size of the **C** allocation.  `sizeof(celt_sig) ==
/// sizeof(celt_glog) == 4` in this build.
pub fn opus_custom_encoder_get_size(mode: &CELTMode, channels: i32) -> i32 {
    /* `ENABLE_QEXT` is undefined, so `extra == 0` and `QEXT_SCALE(x) == x`. */
    SIZEOF_CELT_ENCODER
        + (channels * mode.overlap - 1) * 4 /* celt_sig in_mem[channels*mode->overlap] */
        + channels * COMBFILTER_MAXPERIOD * 4 /* celt_sig prefilter_mem[] */
        + 4 * channels * mode.nbEBands * 4 /* oldBandE/oldLogE/oldLogE2/energyError */
}

/* ------------------------------------------------------------------------- *
 * opus_custom_encoder_init / create / destroy, celt_encoder_init
 * ------------------------------------------------------------------------- */

/// The body of `opus_custom_encoder_ctl(st, OPUS_RESET_STATE)`: the C clears
/// everything from `ENCODER_RESET_START` (`rng`) to the end of the allocation,
/// then re-initialises a handful of fields.
fn celt_encoder_reset_state(st: &mut OpusCustomEncoder) {
    st.rng = 0;
    st.spread_decision = 0;
    st.delayedIntra = 0.0;
    st.tonal_average = 0;
    st.lastCodedBands = 0;
    st.hf_average = 0;
    st.tapset_decision = 0;
    st.prefilter_period = 0;
    st.prefilter_gain = 0.0;
    st.prefilter_tapset = 0;
    st.consec_transient = 0;
    st.analysis = AnalysisInfo::default();
    st.silk_info = SILKInfo::default();
    st.preemph_memE = [0.0; 2];
    st.preemph_memD = [0.0; 2];
    st.vbr_reservoir = 0;
    st.vbr_drift = 0;
    st.vbr_offset = 0;
    st.vbr_count = 0;
    st.overlap_max = 0.0;
    st.stereo_saving = 0.0;
    st.intensity = 0;
    st.energy_mask = None;
    st.spec_avg = 0.0;
    /* the trailing allocation */
    for v in st.in_mem.iter_mut() {
        *v = 0.0;
    }
    for v in st.prefilter_mem.iter_mut() {
        *v = 0.0;
    }
    for v in st.oldBandE.iter_mut() {
        *v = 0.0;
    }
    for v in st.energyError.iter_mut() {
        *v = 0.0;
    }
    for v in st.oldLogE.iter_mut() {
        *v = -GCONST(28.0f32);
    }
    for v in st.oldLogE2.iter_mut() {
        *v = -GCONST(28.0f32);
    }
    st.vbr_offset = 0;
    st.delayedIntra = 1.0;
    st.spread_decision = super::bands::SPREAD_NORMAL;
    st.tonal_average = 256;
    st.hf_average = 0;
    st.tapset_decision = 0;
}

/// `opus_custom_encoder_init_arch()` (`celt/celt_encoder.c:194`).
pub fn opus_custom_encoder_init_arch(
    st: &mut OpusCustomEncoder,
    mode: Box<CELTMode>,
    channels: i32,
    arch: i32,
) -> i32 {
    if channels < 0 || channels > 2 {
        return OPUS_BAD_ARG;
    }
    /* C: `if (st==NULL || mode==NULL) return OPUS_ALLOC_FAIL;`.  Rust cannot
       pass a NULL `&mut`, and `mode` is an owned `Box`. */

    /* `OPUS_CLEAR((char*)st, opus_custom_encoder_get_size(mode, channels));` */
    let nbEBands = mode.nbEBands;
    let overlap = mode.overlap;
    *st = OpusCustomEncoder::zeroed();
    st.mode = mode;
    st.stream_channels = channels;
    st.channels = channels;

    st.upsample = 1;
    st.start = 0;
    st.end = st.mode.effEBands;
    st.signalling = 1;
    st.arch = arch;

    st.constrained_vbr = 1;
    st.clip = 1;

    st.bitrate = OPUS_BITRATE_MAX;
    st.vbr = 0;
    st.force_intra = 0;
    st.complexity = 5;
    st.lsb_depth = 24;

    /* the trailing arrays of the single allocation */
    st.in_mem = vec![0.0; (channels * overlap) as usize];
    st.prefilter_mem = vec![0.0; (channels * COMBFILTER_MAXPERIOD) as usize];
    st.oldBandE = vec![0.0; (channels * nbEBands) as usize];
    st.oldLogE = vec![0.0; (channels * nbEBands) as usize];
    st.oldLogE2 = vec![0.0; (channels * nbEBands) as usize];
    st.energyError = vec![0.0; (channels * nbEBands) as usize];

    opus_custom_encoder_ctl(st, OPUS_RESET_STATE, &mut 0);
    OPUS_OK
}

/// `opus_custom_encoder_create()` (`celt/celt_encoder.c:177`).
pub fn opus_custom_encoder_create(
    mode: Box<CELTMode>,
    channels: i32,
    error: &mut i32,
) -> Option<Box<OpusCustomEncoder>> {
    let mut st = Box::new(OpusCustomEncoder::zeroed());
    let ret = opus_custom_encoder_init_arch(&mut st, mode, channels, OPUS_ARCH);
    if ret != OPUS_OK {
        *error = ret;
        return None;
    }
    *error = ret;
    Some(st)
}

/// `opus_custom_encoder_init()` (`celt/celt_encoder.c:234`).
pub fn opus_custom_encoder_init(
    st: &mut OpusCustomEncoder,
    mode: Box<CELTMode>,
    channels: i32,
) -> i32 {
    /* `opus_select_arch()` == 4 on the reference host. */
    opus_custom_encoder_init_arch(st, mode, channels, OPUS_ARCH)
}

/// `opus_custom_encoder_destroy()` (`celt/celt_encoder.c:260`).  Ownership of
/// the state (and of its mode) lives in the `Box`, so dropping it is the
/// deallocation.
pub fn opus_custom_encoder_destroy(st: Box<OpusCustomEncoder>) {
    drop(st);
}

/// `celt_encoder_init()` (`celt/celt_encoder.c:240`).
pub fn celt_encoder_init(
    st: &mut OpusCustomEncoder,
    sampling_rate: opus_int32,
    channels: i32,
    arch: i32,
) -> i32 {
    /* `ENABLE_QEXT` is undefined: only the 48 kHz mode is created. */
    let mut err = 0i32;
    let mode = match opus_custom_mode_create(48000, 960, &mut err) {
        Some(m) => m,
        None => return OPUS_ALLOC_FAIL,
    };
    let ret = opus_custom_encoder_init_arch(st, mode, channels, arch);
    if ret != OPUS_OK {
        return ret;
    }
    st.upsample = resampling_factor(sampling_rate);
    OPUS_OK
}

/// Rust convenience wrapper: `opus_alloc(celt_encoder_get_size())` followed by
/// `celt_encoder_init()`.
pub fn celt_encoder_create(
    sampling_rate: opus_int32,
    channels: i32,
    arch: i32,
) -> Option<Box<OpusCustomEncoder>> {
    let mut st = Box::new(OpusCustomEncoder::zeroed());
    let ret = celt_encoder_init(&mut st, sampling_rate, channels, arch);
    if ret != OPUS_OK {
        None
    } else {
        Some(st)
    }
}

/* ------------------------------------------------------------------------- *
 * opus_custom_encoder_ctl
 * ------------------------------------------------------------------------- */

/// `opus_custom_encoder_ctl()` (`celt/celt_encoder.c:2941`) for the requests
/// whose C argument is a scalar `opus_int32` (read or written through `value`).
///
/// The `CELT_SET_ANALYSIS`, `CELT_SET_SILK_INFO`, `OPUS_SET_ENERGY_MASK` and
/// `CELT_GET_MODE` requests take pointers in C and are provided by the
/// `opus_custom_encoder_ctl_*` functions below.
pub fn opus_custom_encoder_ctl(
    st: &mut OpusCustomEncoder,
    request: i32,
    value: &mut i32,
) -> i32 {
    match request {
        OPUS_SET_COMPLEXITY_REQUEST => {
            if *value < 0 || *value > 10 {
                return OPUS_BAD_ARG;
            }
            st.complexity = *value;
        }
        super::celt::CELT_SET_START_BAND_REQUEST => {
            if *value < 0 || *value >= st.mode.nbEBands {
                return OPUS_BAD_ARG;
            }
            st.start = *value;
        }
        super::celt::CELT_SET_END_BAND_REQUEST => {
            if *value < 1 || *value > st.mode.nbEBands {
                return OPUS_BAD_ARG;
            }
            st.end = *value;
        }
        super::celt::CELT_SET_PREDICTION_REQUEST => {
            if *value < 0 || *value > 2 {
                return OPUS_BAD_ARG;
            }
            st.disable_pf = (*value <= 1) as i32;
            st.force_intra = (*value == 0) as i32;
        }
        OPUS_SET_PACKET_LOSS_PERC_REQUEST => {
            if *value < 0 || *value > 100 {
                return OPUS_BAD_ARG;
            }
            st.loss_rate = *value;
        }
        OPUS_SET_VBR_CONSTRAINT_REQUEST => {
            st.constrained_vbr = *value;
        }
        OPUS_SET_VBR_REQUEST => {
            st.vbr = *value;
        }
        OPUS_SET_BITRATE_REQUEST => {
            if *value <= 500 && *value != OPUS_BITRATE_MAX {
                return OPUS_BAD_ARG;
            }
            let v = IMIN(*value, 750000 * st.channels);
            st.bitrate = v;
        }
        super::celt::CELT_SET_CHANNELS_REQUEST => {
            if *value < 1 || *value > 2 {
                return OPUS_BAD_ARG;
            }
            st.stream_channels = *value;
        }
        OPUS_SET_LSB_DEPTH_REQUEST => {
            if *value < 8 || *value > 24 {
                return OPUS_BAD_ARG;
            }
            st.lsb_depth = *value;
        }
        OPUS_GET_LSB_DEPTH_REQUEST => {
            *value = st.lsb_depth;
        }
        OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST => {
            if *value < 0 || *value > 1 {
                return OPUS_BAD_ARG;
            }
            st.disable_inv = *value;
        }
        OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST => {
            *value = st.disable_inv;
        }
        OPUS_RESET_STATE => {
            celt_encoder_reset_state(st);
        }
        super::celt::CELT_SET_INPUT_CLIPPING_REQUEST => {
            st.clip = *value;
        }
        super::celt::CELT_SET_SIGNALLING_REQUEST => {
            st.signalling = *value;
        }
        OPUS_GET_FINAL_RANGE_REQUEST => {
            *value = st.rng as i32;
        }
        super::celt::OPUS_SET_LFE_REQUEST => {
            st.lfe = *value;
        }
        _ => return OPUS_UNIMPLEMENTED,
    }
    OPUS_OK
}

/// `opus_custom_encoder_ctl(st, CELT_SET_ANALYSIS_REQUEST, info)`.
///
/// C: `if (info) OPUS_COPY(&st->analysis, info, 1);` — a NULL `info` is a
/// no-op and still returns `OPUS_OK`.
pub fn opus_custom_encoder_ctl_analysis(
    st: &mut OpusCustomEncoder,
    info: Option<&AnalysisInfo>,
) -> i32 {
    if let Some(i) = info {
        st.analysis = *i;
    }
    OPUS_OK
}

/// `opus_custom_encoder_ctl(st, CELT_SET_SILK_INFO_REQUEST, info)`.
pub fn opus_custom_encoder_ctl_silk_info(
    st: &mut OpusCustomEncoder,
    info: Option<&SILKInfo>,
) -> i32 {
    if let Some(i) = info {
        st.silk_info = *i;
    }
    OPUS_OK
}

/// `opus_custom_encoder_ctl(st, OPUS_SET_ENERGY_MASK_REQUEST, mask)`.
///
/// C stores the caller's pointer (NULL disables the mask); the Rust port
/// copies the values, so the mask must be `nbEBands * channels` long.
pub fn opus_custom_encoder_ctl_energy_mask(
    st: &mut OpusCustomEncoder,
    mask: Option<&[celt_glog]>,
) -> i32 {
    st.energy_mask = mask.map(|m| m.to_vec());
    OPUS_OK
}

/// `opus_custom_encoder_ctl(st, CELT_GET_MODE_REQUEST, &mode)`.
pub fn opus_custom_encoder_get_mode(st: &OpusCustomEncoder) -> &CELTMode {
    &st.mode
}

/// `#define celt_encoder_ctl opus_custom_encoder_ctl` (`celt/celt.h:184`).
pub use opus_custom_encoder_ctl as celt_encoder_ctl;

/* ------------------------------------------------------------------------- *
 * celt_preemphasis
 * ------------------------------------------------------------------------- */

/// `celt_preemphasis()` (`celt/celt_encoder.c:557`).
///
/// `pcmp` is the interleaved PCM in the `opus_res` domain, `inp` the output
/// (only `N` samples are written), and `mem` is `st->preemph_memE + c`.
pub fn celt_preemphasis(
    pcmp: &[opus_res],
    inp: &mut [celt_sig],
    N: i32,
    CC: i32,
    upsample: i32,
    coef: &[opus_val16],
    mem: &mut celt_sig,
    clip: i32,
) {
    let coef0 = coef[0];
    let mut m = *mem;

    /* Fast path for the normal 48kHz case and no clipping */
    if coef[1] == 0.0 && upsample == 1 && clip == 0 {
        for i in 0..N as usize {
            let x = RES2SIG(pcmp[CC as usize * i]);
            /* Apply pre-emphasis */
            inp[i] = x - m;
            m = MULT16_32_Q15(coef0, x);
        }
        *mem = m;
        return;
    }

    let Nu = N / upsample;
    if upsample != 1 {
        for v in inp[..N as usize].iter_mut() {
            *v = 0.0;
        }
    }
    for i in 0..Nu as usize {
        inp[i * upsample as usize] = RES2SIG(pcmp[CC as usize * i]);
    }

    /* `#ifndef FIXED_POINT` */
    if clip != 0 {
        /* Clip input to avoid encoding non-portable files */
        for i in 0..Nu as usize {
            let k = i * upsample as usize;
            inp[k] = MAX32(-65536.0f32, MIN32(65536.0f32, inp[k]));
        }
    }

    /* `#if defined(CUSTOM_MODES) || ...` — CUSTOM_MODES is defined. */
    if coef[1] != 0.0 {
        let coef1 = coef[1];
        let coef2 = coef[2];
        for i in 0..N as usize {
            let x = inp[i];
            /* Apply pre-emphasis */
            let tmp = SHL32(MULT16_32_Q15(coef2, x), 15 - SIG_SHIFT);
            inp[i] = tmp + m;
            m = MULT16_32_Q15(coef1, inp[i]) - MULT16_32_Q15(coef0, tmp);
        }
    } else {
        for i in 0..N as usize {
            let x = inp[i];
            /* Apply pre-emphasis */
            inp[i] = x - m;
            m = MULT16_32_Q15(coef0, x);
        }
    }
    *mem = m;
}

/* ------------------------------------------------------------------------- *
 * compute_mdcts
 * ------------------------------------------------------------------------- */

/// `compute_mdcts()` (`celt/celt_encoder.c:511`).
fn compute_mdcts(
    mode: &CELTMode,
    shortBlocks: i32,
    in_: &[celt_sig],
    out: &mut [celt_sig],
    C: i32,
    CC: i32,
    LM: i32,
    upsample: i32,
    arch: i32,
) {
    let overlap = mode.overlap;
    let N;
    let B;
    let shift;
    if shortBlocks != 0 {
        B = shortBlocks;
        N = mode.shortMdctSize;
        shift = mode.maxLM;
    } else {
        B = 1;
        N = mode.shortMdctSize << LM;
        shift = mode.maxLM - LM;
    }
    for c in 0..CC {
        for b in 0..B {
            /* Interleaving the sub-frames while doing the MDCTs */
            let ioff = (c * (B * N + overlap) + b * N) as usize;
            let ooff = (b + c * N * B) as usize;
            clt_mdct_forward(
                &mode.mdct,
                &in_[ioff..],
                &mut out[ooff..],
                &mode.window,
                overlap,
                shift,
                B,
                arch,
            );
        }
    }
    if CC == 2 && C == 1 {
        for i in 0..(B * N) as usize {
            out[i] = ADD32(HALF32(out[i]), HALF32(out[(B * N) as usize + i]));
        }
    }
    if upsample != 1 {
        for c in 0..C {
            let bound = B * N / upsample;
            for i in 0..bound as usize {
                out[(c * B * N) as usize + i] *= upsample as f32;
            }
            for i in bound..B * N {
                out[(c * B * N + i) as usize] = 0.0;
            }
        }
    }
}

/* ------------------------------------------------------------------------- *
 * transient_analysis
 * ------------------------------------------------------------------------- */

/// `transient_analysis()` (`celt/celt_encoder.c:267`) — float body.
fn transient_analysis(
    in_: &[opus_val32],
    len: i32,
    C: i32,
    tf_estimate: &mut opus_val16,
    tf_chan: &mut i32,
    allow_weak_transients: i32,
    weak_transient: &mut i32,
    tone_freq: opus_val16,
    toneishness: opus_val32,
) -> i32 {
    /* Table of 6*64/x, trained on real data to minimize the average error */
    static inv_table: [u8; 128] = [
        255, 255, 156, 110, 86, 70, 59, 51, 45, 40, 37, 33, 31, 28, 26, 25, 23, 22, 21, 20, 19,
        18, 17, 16, 16, 15, 15, 14, 13, 13, 12, 12, 12, 12, 11, 11, 11, 10, 10, 10, 9, 9, 9, 9, 9,
        9, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 5,
        5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
        4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2,
    ];

    let mut tmp = vec![0.0f32; len as usize];
    let mut is_transient;
    let mut mask_metric: opus_int32 = 0;
    let mut forward_decay = QCONST16(0.0625f32, 15);
    let len2 = len / 2;

    *weak_transient = 0;
    if allow_weak_transients != 0 {
        forward_decay = QCONST16(0.03125f32, 15);
    }

    for c in 0..C {
        let mut mean: opus_val32;
        let mut unmask: opus_int32 = 0;
        let mut norm: opus_val32;
        let mut maxE: opus_val16;
        let mut mem0: opus_val32 = 0.0;
        let mut mem1: opus_val32 = 0.0;
        /* High-pass filter: (1 - 2*z^-1 + z^-2) / (1 - z^-1 + .5*z^-2) */
        for i in 0..len as usize {
            let x = SHR32(in_[i + c as usize * len as usize], 0);
            let y = ADD32(mem0, x);
            /* Modified code to shorten dependency chains: */
            let mem00 = mem0;
            mem0 = mem0 - x + 0.5f32 * mem1;
            mem1 = x - mem00;
            tmp[i] = SROUND16(y, 2);
        }
        /* First few samples are bad because we don't propagate the memory */
        for v in tmp[..12].iter_mut() {
            *v = 0.0;
        }

        mean = 0.0;
        mem0 = 0.0;
        /* Grouping by two to reduce complexity */
        /* Forward pass to compute the post-echo threshold*/
        for i in 0..len2 as usize {
            let x2 = PSHR32(
                MULT16_16(tmp[2 * i], tmp[2 * i]) + MULT16_16(tmp[2 * i + 1], tmp[2 * i + 1]),
                4,
            );
            mean += PSHR32(x2, 12);
            mem0 = x2 + (1.0f32 - forward_decay) * mem0;
            tmp[i] = forward_decay * mem0;
        }

        mem0 = 0.0;
        maxE = 0.0;
        /* Backward pass to compute the pre-echo threshold */
        let mut i = len2 - 1;
        while i >= 0 {
            /* Backward masking: 13.9 dB/ms. */
            mem0 = tmp[i as usize] + 0.875f32 * mem0;
            tmp[i as usize] = 0.125f32 * mem0;
            maxE = MAX16(maxE, 0.125f32 * mem0);
            i -= 1;
        }

        /* As a compromise with the old transient detector, frame energy is the
           geometric mean of the energy and half the max */
        mean = celt_sqrt(mean * maxE * 0.5f32 * len2 as f32);
        /* Inverse of the mean energy in Q15+6 */
        norm = SHL32(EXTEND32(len2 as f32), 6 + 14) / ADD32(EPSILON, SHR32(mean, 1));
        /* Compute harmonic mean discarding the unreliable boundaries
           The data is smooth, so we only take 1/4th of the samples */
        unmask = 0;
        celt_assert!(!celt_isnan(tmp[0]));
        celt_assert!(!celt_isnan(norm));
        let mut i = 12;
        while i < len2 - 5 {
            let v = (64.0f32 * norm * (tmp[i as usize] + EPSILON)) as f64;
            /* `(int)MAX32(0,MIN32(127,floor(v)))` */
            let id: i32 = if v < 0.0 {
                0
            } else if v > 127.0 {
                127
            } else {
                v.floor() as i32
            };
            unmask += inv_table[id as usize] as i32;
            i += 4;
        }
        /* Normalize, compensate for the 1/4th of the sample and the factor of 6 in the inverse table */
        unmask = 64 * unmask * 4 / (6 * (len2 - 17));
        if unmask > mask_metric {
            *tf_chan = c;
            mask_metric = unmask;
        }
    }
    is_transient = (mask_metric > 200) as i32;
    /* Prevent the transient detector from confusing the partial cycle of a
       very low frequency tone with a transient. */
    if toneishness > QCONST32(0.98f32, 29) && tone_freq < QCONST16(0.026f32, 13) {
        is_transient = 0;
        mask_metric = 0;
    }
    /* For low bitrates, define "weak transients" that need to be
       handled differently to avoid partial collapse. */
    if allow_weak_transients != 0 && is_transient != 0 && mask_metric < 600 {
        is_transient = 0;
        *weak_transient = 1;
    }
    /* Arbitrary metric for VBR boost */
    let tf_max = MAX16(0.0f32, celt_sqrt(27.0f32 * mask_metric as f32) - 42.0f32);
    /* *tf_estimate = 1 + MIN16(1, sqrt(MAX16(0, tf_max-30))/20); */
    *tf_estimate = celt_sqrt(MAX32(
        0.0f32,
        SHL32(
            MULT16_16(
                QCONST16(0.0069f32, 14),
                MIN16(163.0f32, tf_max),
            ),
            14,
        ) - QCONST32(0.139f32, 28),
    ));
    is_transient
}

/* ------------------------------------------------------------------------- *
 * patch_transient_decision
 * ------------------------------------------------------------------------- */

/// `patch_transient_decision()` (`celt/celt_encoder.c:473`).
fn patch_transient_decision(
    newE: &[celt_glog],
    oldE: &[celt_glog],
    nbEBands: i32,
    start: i32,
    end: i32,
    C: i32,
) -> i32 {
    let mut mean_diff: opus_val32 = 0.0;
    let mut spread_old = [0.0f32; 26];
    /* Apply an aggressive (-6 dB/Bark) spreading function to the old frame to
       avoid false detection caused by irrelevant bands */
    if C == 1 {
        spread_old[start as usize] = oldE[start as usize];
        for i in start + 1..end {
            spread_old[i as usize] = MAXG(spread_old[(i - 1) as usize] - GCONST(1.0f32), oldE[i as usize]);
        }
    } else {
        spread_old[start as usize] = MAXG(oldE[start as usize], oldE[(start + nbEBands) as usize]);
        for i in start + 1..end {
            spread_old[i as usize] = MAXG(
                spread_old[(i - 1) as usize] - GCONST(1.0f32),
                MAXG(oldE[i as usize], oldE[(i + nbEBands) as usize]),
            );
        }
    }
    let mut i = end - 2;
    while i >= start {
        spread_old[i as usize] = MAXG(spread_old[i as usize], spread_old[(i + 1) as usize] - GCONST(1.0f32));
        i -= 1;
    }
    /* Compute mean increase */
    let mut c = 0;
    loop {
        let mut i = IMAX(2, start);
        while i < end - 1 {
            let x1 = MAXG(0.0f32, newE[(i + c * nbEBands) as usize]);
            let x2 = MAXG(0.0f32, spread_old[i as usize]);
            mean_diff = ADD32(mean_diff, MAXG(0.0f32, SUB32(x1, x2)));
            i += 1;
        }
        c += 1;
        if c >= C {
            break;
        }
    }
    mean_diff = DIV32(mean_diff, (C * (end - 1 - IMAX(2, start))) as opus_val32);
    (mean_diff > GCONST(1.0f32)) as i32
}

/* ------------------------------------------------------------------------- *
 * l1_metric / tf_analysis / tf_encode
 * ------------------------------------------------------------------------- */

/// `l1_metric()` (`celt/celt_encoder.c:650`).
fn l1_metric(tmp: &[celt_norm], N: i32, LM: i32, bias: opus_val16) -> opus_val32 {
    let mut L1: opus_val32 = 0.0;
    for i in 0..N as usize {
        L1 += EXTEND32(ABS16(SHR32(tmp[i], NORM_SHIFT - 14)));
    }
    /* When in doubt, prefer good freq resolution */
    L1 = MAC16_32_Q15(L1, LM as opus_val32 * bias, L1);
    L1
}

/// `tf_analysis()` (`celt/celt_encoder.c:663`).
fn tf_analysis(
    m: &CELTMode,
    len: i32,
    isTransient: i32,
    tf_res: &mut [i32],
    lambda: i32,
    X: &[celt_norm],
    N0: i32,
    LM: i32,
    tf_estimate: opus_val16,
    tf_chan: i32,
    importance: &[i32],
) -> i32 {
    let mut metric = vec![0i32; len as usize];
    let tmp_len = (((m.eBands[len as usize] as i32) - (m.eBands[(len - 1) as usize] as i32)) << LM)
        as usize;
    let mut tmp = vec![0.0f32 as celt_norm; tmp_len];
    let mut tmp_1 = vec![0.0f32 as celt_norm; tmp_len];
    let mut path0 = vec![0i32; len as usize];
    let mut path1 = vec![0i32; len as usize];
    let mut selcost = [0i32; 2];
    let mut tf_select = 0i32;

    let bias = MULT16_16_Q14(
        QCONST16(0.04f32, 15),
        MAX16(-QCONST16(0.25f32, 14), QCONST16(0.5f32, 14) - tf_estimate),
    );

    for i in 0..len {
        let narrow;
        let mut best_level = 0i32;
        let N = ((m.eBands[(i + 1) as usize] as i32) - (m.eBands[i as usize] as i32)) << LM;
        /* band is too narrow to be split down to LM=-1 */
        narrow = (m.eBands[(i + 1) as usize] - m.eBands[i as usize]) == 1;
        let xoff = (tf_chan * N0 + ((m.eBands[i as usize] as i32) << LM)) as usize;
        tmp[..N as usize].copy_from_slice(&X[xoff..xoff + N as usize]);
        let mut L1 = l1_metric(&tmp[..N as usize], N, if isTransient != 0 { LM } else { 0 }, bias);
        let mut best_L1 = L1;
        /* Check the -1 case for transients */
        if isTransient != 0 && !narrow {
            tmp_1[..N as usize].copy_from_slice(&tmp[..N as usize]);
            haar1(&mut tmp_1[..N as usize], N >> LM, 1 << LM);
            L1 = l1_metric(&tmp_1[..N as usize], N, LM + 1, bias);
            if L1 < best_L1 {
                best_L1 = L1;
                best_level = -1;
            }
        }
        let kmax = LM + (if (isTransient != 0) || narrow { 0 } else { 1 });
        for k in 0..kmax {
            let B = if isTransient != 0 { LM - k - 1 } else { k + 1 };
            haar1(&mut tmp[..N as usize], N >> k, 1 << k);
            L1 = l1_metric(&tmp[..N as usize], N, B, bias);
            if L1 < best_L1 {
                best_L1 = L1;
                best_level = k + 1;
            }
        }
        /* metric is in Q1 to be able to select the mid-point (-0.5) for narrower bands */
        if isTransient != 0 {
            metric[i as usize] = 2 * best_level;
        } else {
            metric[i as usize] = -2 * best_level;
        }
        /* For bands that can't be split to -1, set the metric to the half-way point to avoid
           biasing the decision */
        if narrow && (metric[i as usize] == 0 || metric[i as usize] == -2 * LM) {
            metric[i as usize] -= 1;
        }
    }
    /* Search for the optimal tf resolution, including tf_select */
    tf_select = 0;
    for sel in 0..2 {
        let mut cost0 = importance[0]
            * (metric[0] - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * sel) as usize] as i32))
                .abs();
        let mut cost1 = importance[0]
            * (metric[0] - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * sel + 1) as usize] as i32))
                .abs()
            + (if isTransient != 0 { 0 } else { lambda });
        for i in 1..len {
            let curr0 = IMIN(cost0, cost1 + lambda);
            let curr1 = IMIN(cost0 + lambda, cost1);
            cost0 = curr0
                + importance[i as usize]
                    * (metric[i as usize]
                        - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * sel) as usize] as i32))
                        .abs();
            cost1 = curr1
                + importance[i as usize]
                    * (metric[i as usize]
                        - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * sel + 1) as usize] as i32))
                        .abs();
        }
        cost0 = IMIN(cost0, cost1);
        selcost[sel as usize] = cost0;
    }
    /* For now, we're conservative and only allow tf_select=1 for transients.
     * If tests confirm it's useful for non-transients, we could allow it. */
    if selcost[1] < selcost[0] && isTransient != 0 {
        tf_select = 1;
    }
    let mut cost0 = importance[0]
        * (metric[0] - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * tf_select) as usize] as i32))
            .abs();
    let mut cost1 = importance[0]
        * (metric[0] - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * tf_select + 1) as usize] as i32))
            .abs()
        + (if isTransient != 0 { 0 } else { lambda });
    /* Viterbi forward pass */
    for i in 1..len {
        let curr0;
        let curr1;

        let from0 = cost0;
        let from1 = cost1 + lambda;
        if from0 < from1 {
            curr0 = from0;
            path0[i as usize] = 0;
        } else {
            curr0 = from1;
            path0[i as usize] = 1;
        }

        let from0 = cost0 + lambda;
        let from1 = cost1;
        if from0 < from1 {
            curr1 = from0;
            path1[i as usize] = 0;
        } else {
            curr1 = from1;
            path1[i as usize] = 1;
        }
        cost0 = curr0
            + importance[i as usize]
                * (metric[i as usize]
                    - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * tf_select) as usize] as i32))
                    .abs();
        cost1 = curr1
            + importance[i as usize]
                * (metric[i as usize]
                    - 2 * (tf_select_table[LM as usize][(4 * isTransient + 2 * tf_select + 1) as usize] as i32))
                    .abs();
    }
    tf_res[(len - 1) as usize] = if cost0 < cost1 { 0 } else { 1 };
    /* Viterbi backward pass to check the decisions */
    let mut i = len - 2;
    while i >= 0 {
        if tf_res[(i + 1) as usize] == 1 {
            tf_res[i as usize] = path1[(i + 1) as usize];
        } else {
            tf_res[i as usize] = path0[(i + 1) as usize];
        }
        i -= 1;
    }
    tf_select
}

/// `tf_encode()` (`celt/celt_encoder.c:824`).
fn tf_encode(
    start: i32,
    end: i32,
    isTransient: i32,
    tf_res: &mut [i32],
    LM: i32,
    mut tf_select: i32,
    enc: &mut ec_ctx,
) {
    let mut budget: opus_uint32 = enc.storage.wrapping_mul(8);
    let mut tell: opus_uint32 = enc.ec_tell() as opus_uint32;
    let mut logp: opus_uint32 = if isTransient != 0 { 2 } else { 4 };
    let mut curr: i32;
    let mut tf_changed: i32;
    /* Reserve space to code the tf_select decision. */
    let tf_select_rsv: i32 = (LM > 0 && tell + logp + 1 <= budget) as i32;
    budget = budget.wrapping_sub(tf_select_rsv as opus_uint32);
    curr = 0;
    tf_changed = 0;
    for i in start..end {
        if tell + logp <= budget {
            ec_enc_bit_logp(enc, tf_res[i as usize] ^ curr, logp);
            tell = enc.ec_tell() as opus_uint32;
            curr = tf_res[i as usize];
            tf_changed |= curr;
        } else {
            tf_res[i as usize] = curr;
        }
        logp = if isTransient != 0 { 4 } else { 5 };
    }
    /* Only code tf_select if it would actually make a difference. */
    if tf_select_rsv != 0
        && tf_select_table[LM as usize][(4 * isTransient + 0 + tf_changed) as usize]
            != tf_select_table[LM as usize][(4 * isTransient + 2 + tf_changed) as usize]
    {
        ec_enc_bit_logp(enc, tf_select, 1);
    } else {
        tf_select = 0;
    }
    for i in start..end {
        tf_res[i as usize] =
            tf_select_table[LM as usize][(4 * isTransient + 2 * tf_select + tf_res[i as usize]) as usize]
                as i32;
    }
}

/* ------------------------------------------------------------------------- *
 * alloc_trim_analysis / stereo_analysis / median filters
 * ------------------------------------------------------------------------- */

/// `alloc_trim_analysis()` (`celt/celt_encoder.c:865`).
#[allow(clippy::too_many_arguments)]
fn alloc_trim_analysis(
    m: &CELTMode,
    X: &[celt_norm],
    bandLogE: &[celt_glog],
    end: i32,
    LM: i32,
    C: i32,
    N0: i32,
    analysis: &AnalysisInfo,
    stereo_saving: &mut opus_val16,
    tf_estimate: opus_val16,
    intensity: i32,
    surround_trim: celt_glog,
    equiv_rate: opus_int32,
    arch: i32,
) -> i32 {
    let _ = arch;
    let mut diff: opus_val32 = 0.0;
    let mut trim_index;
    let mut trim = QCONST16(5.0f32, 8);
    let logXC;
    let logXC2;
    /* At low bitrate, reducing the trim seems to help. At higher bitrates, it's less
       clear what's best, so we're keeping it as it was before, at least for now. */
    if equiv_rate < 64000 {
        trim = QCONST16(4.0f32, 8);
    } else if equiv_rate < 80000 {
        let frac = (equiv_rate - 64000) >> 10;
        trim = QCONST16(4.0f32, 8) + QCONST16(1.0f32 / 16.0f32, 8) * frac as opus_val16;
    }
    if C == 2 {
        let mut sum: opus_val16 = 0.0; /* Q10 */
        /* Compute inter-channel correlation for low frequencies */
        for i in 0..8 {
            let off = (m.eBands[i as usize] as i32) << LM;
            let partial =
                celt_inner_prod(&X[off as usize..], &X[(N0 + off) as usize..], ((m.eBands[(i + 1) as usize] as i32) - (m.eBands[i as usize] as i32)) << LM);
            sum = ADD16(sum, EXTRACT16(SHR32(partial, 18)));
        }
        sum = MULT16_16_Q15(QCONST16(1.0f32 / 8.0f32, 15), sum);
        sum = MIN16(QCONST16(1.0f32, 10), ABS16(sum));
        let mut minXC = sum;
        for i in 8..intensity {
            let off = (m.eBands[i as usize] as i32) << LM;
            let partial =
                celt_inner_prod(&X[off as usize..], &X[(N0 + off) as usize..], ((m.eBands[(i + 1) as usize] as i32) - (m.eBands[i as usize] as i32)) << LM);
            minXC = MIN16(minXC, ABS16(EXTRACT16(SHR32(partial, 18))));
        }
        minXC = MIN16(QCONST16(1.0f32, 10), ABS16(minXC));
        /* mid-side savings estimations based on the LF average*/
        logXC = celt_log2(QCONST32(1.001f32, 20) - MULT16_16(sum, sum));
        /* mid-side savings estimations based on min correlation */
        logXC2 = MAX16(
            HALF16(logXC),
            celt_log2(QCONST32(1.001f32, 20) - MULT16_16(minXC, minXC)),
        );

        trim += MAX16(
            -QCONST16(4.0f32, 8),
            MULT16_16_Q15(QCONST16(0.75f32, 15), logXC),
        );
        *stereo_saving = MIN16(
            *stereo_saving + QCONST16(0.25f32, 8),
            -HALF16(logXC2),
        );
    }

    /* Estimate spectral tilt */
    let mut c = 0;
    loop {
        for i in 0..end - 1 {
            diff += SHR32(bandLogE[(i + c * m.nbEBands) as usize], 5)
                * (2 + 2 * i - end) as opus_int32 as opus_val32;
        }
        c += 1;
        if c >= C {
            break;
        }
    }
    diff /= (C * (end - 1)) as opus_val32;
    trim -= MAX32(
        -QCONST16(2.0f32, 8),
        MIN32(
            QCONST16(2.0f32, 8),
            SHR32(diff + QCONST32(1.0f32, DB_SHIFT - 5), DB_SHIFT - 13) / 6.0f32,
        ),
    );
    trim -= SHR16(surround_trim, DB_SHIFT - 8);
    trim -= 2.0f32 * SHR16(tf_estimate, 14 - 8);
    /* `#ifndef DISABLE_FLOAT_API` */
    if analysis.valid != 0 {
        trim -= MAX16(
            -QCONST16(2.0f32, 8),
            MIN16(
                QCONST16(2.0f32, 8),
                QCONST16(2.0f32, 8) * (analysis.tonality_slope + 0.05f32),
            ),
        );
    }

    /* `#ifdef FIXED_POINT` is not compiled: `floor(.5f+trim)` in double. */
    trim_index = ((0.5f32 + trim) as f64).floor() as i32;
    trim_index = IMAX(0, IMIN(10, trim_index));
    trim_index
}

/// `stereo_analysis()` (`celt/celt_encoder.c:957`).
fn stereo_analysis(m: &CELTMode, X: &[celt_norm], LM: i32, N0: i32) -> i32 {
    let mut sumLR: opus_val32 = EPSILON;
    let mut sumMS: opus_val32 = EPSILON;

    /* Use the L1 norm to model the entropy of the L/R signal vs the M/S signal */
    for i in 0..13 {
        for j in (m.eBands[i as usize] as i32) << LM..(m.eBands[(i + 1) as usize] as i32) << LM {
            /* We cast to 32-bit first because of the -32768 case */
            let L = SHR32(X[j as usize], NORM_SHIFT - 14);
            let R = SHR32(X[(N0 + j) as usize], NORM_SHIFT - 14);
            let M = ADD32(L, R);
            let S = SUB32(L, R);
            sumLR = ADD32(sumLR, ADD32(ABS32(L), ABS32(R)));
            sumMS = ADD32(sumMS, ADD32(ABS32(M), ABS32(S)));
        }
    }
    sumMS = MULT16_32_Q15(QCONST16(0.707107f32, 15), sumMS);
    let mut thetas = 13;
    /* We don't need thetas for lower bands with LM<=1 */
    if LM <= 1 {
        thetas -= 8;
    }
    (MULT16_32_Q15(
        (((m.eBands[13] as i32) << (LM + 1)) + thetas) as opus_val32,
        sumMS,
    ) > MULT16_32_Q15(((m.eBands[13] as i32) << (LM + 1)) as opus_val32, sumLR)) as i32
}

/// `median_of_5()` (`celt/celt_encoder.c:990`).
fn median_of_5(x: &[celt_glog]) -> celt_glog {
    let t2 = x[2];
    let mut t0;
    let mut t1;
    let mut t3;
    let mut t4;
    if x[0] > x[1] {
        t0 = x[1];
        t1 = x[0];
    } else {
        t0 = x[0];
        t1 = x[1];
    }
    if x[3] > x[4] {
        t3 = x[4];
        t4 = x[3];
    } else {
        t3 = x[3];
        t4 = x[4];
    }
    if t0 > t3 {
        let tmp = t0;
        t0 = t3;
        t3 = tmp;
        let tmp = t1;
        t1 = t4;
        t4 = tmp;
    }
    if t2 > t1 {
        if t1 < t3 {
            MING(t2, t3)
        } else {
            MING(t4, t1)
        }
    } else if t2 < t3 {
        MING(t1, t3)
    } else {
        MING(t2, t4)
    }
}

/// `median_of_3()` (`celt/celt_encoder.c:1029`).
fn median_of_3(x: &[celt_glog]) -> celt_glog {
    let t0;
    let t1;
    let t2;
    if x[0] > x[1] {
        t0 = x[1];
        t1 = x[0];
    } else {
        t0 = x[0];
        t1 = x[1];
    }
    t2 = x[2];
    if t1 < t2 {
        t1
    } else if t0 < t2 {
        t2
    } else {
        t0
    }
}

/* ------------------------------------------------------------------------- *
 * dynalloc_analysis
 * ------------------------------------------------------------------------- */

/// `dynalloc_analysis()` (`celt/celt_encoder.c:1049`).
#[allow(clippy::too_many_arguments)]
fn dynalloc_analysis(
    bandLogE: &[celt_glog],
    bandLogE2: &[celt_glog],
    oldBandE: &[celt_glog],
    nbEBands: i32,
    start: i32,
    end: i32,
    C: i32,
    offsets: &mut [i32],
    lsb_depth: i32,
    logN: &[opus_int16],
    isTransient: i32,
    vbr: i32,
    constrained_vbr: i32,
    eBands: &[opus_int16],
    LM: i32,
    effectiveBytes: i32,
    tot_boost_: &mut opus_int32,
    lfe: i32,
    surround_dynalloc: &[celt_glog],
    analysis: &AnalysisInfo,
    importance: &mut [i32],
    spread_weight: &mut [i32],
    tone_freq: opus_val16,
    toneishness: opus_val32,
) -> celt_glog {
    let mut c;
    let mut tot_boost: opus_int32 = 0;
    let mut maxDepth: celt_glog;
    let mut follower = vec![0.0f32 as celt_glog; (C * nbEBands) as usize];
    let mut noise_floor = vec![0.0f32 as celt_glog; (C * nbEBands) as usize];
    let mut bandLogE3 = vec![0.0f32 as celt_glog; nbEBands as usize];
    for v in offsets[..nbEBands as usize].iter_mut() {
        *v = 0;
    }
    /* Dynamic allocation code */
    maxDepth = -GCONST(31.9f32);
    for i in 0..end {
        /* Noise floor must take into account eMeans, the depth, the width of the bands
           and the preemphasis filter (approx. square of bark band ID) */
        noise_floor[i as usize] = GCONST(0.0625f32) * logN[i as usize] as opus_val32
            + GCONST(0.5f32)
            + (9 - lsb_depth) as opus_val32
            - SHL32(super::quant_bands::eMeans[i as usize] as opus_val32, DB_SHIFT - 4)
            + GCONST(0.0062f32) * (i + 5) as opus_val32 * (i + 5) as opus_val32;
    }
    c = 0;
    loop {
        for i in 0..end {
            maxDepth = MAXG(maxDepth, bandLogE[(c * nbEBands + i) as usize] - noise_floor[i as usize]);
        }
        c += 1;
        if c >= C {
            break;
        }
    }
    {
        /* Compute a really simple masking model to avoid taking into account completely masked
           bands when computing the spreading decision. */
        let mut mask = vec![0.0f32 as celt_glog; nbEBands as usize];
        let mut sig = vec![0.0f32 as celt_glog; nbEBands as usize];
        for i in 0..end {
            mask[i as usize] = bandLogE[i as usize] - noise_floor[i as usize];
        }
        if C == 2 {
            for i in 0..end {
                mask[i as usize] =
                    MAXG(mask[i as usize], bandLogE[(nbEBands + i) as usize] - noise_floor[i as usize]);
            }
        }
        opus_copy_f32(&mut sig[..end as usize], &mask[..end as usize]);
        for i in 1..end {
            mask[i as usize] = MAXG(mask[i as usize], mask[(i - 1) as usize] - GCONST(2.0f32));
        }
        let mut i = end - 2;
        while i >= 0 {
            mask[i as usize] = MAXG(mask[i as usize], mask[(i + 1) as usize] - GCONST(3.0f32));
            i -= 1;
        }
        for i in 0..end {
            /* Compute SMR: Mask is never more than 72 dB below the peak and never below the noise floor.*/
            let smr = sig[i as usize] - MAXG(MAXG(0.0f32, maxDepth - GCONST(12.0f32)), mask[i as usize]);
            /* Clamp SMR to make sure we're not shifting by something negative or too large. */
            let shift = IMIN(5, IMAX(0, -(((0.5f32 + smr) as f64).floor() as i32)));
            spread_weight[i as usize] = 32 >> shift;
        }
    }
    /* Make sure that dynamic allocation can't make us bust the budget.
       We enable the feature starting at 24 kb/s for 20-ms frames
       and 96 kb/s for 2.5 ms frames.  */
    if effectiveBytes >= (30 + 5 * LM) && lfe == 0 {
        let mut last = 0;
        c = 0;
        loop {
            let offset;
            let mut tmp;
            opus_copy_f32(
                &mut bandLogE3[..end as usize],
                &bandLogE2[(c * nbEBands) as usize..(c * nbEBands + end) as usize],
            );
            if LM == 0 {
                /* For 2.5 ms frames, the first 8 bands have just one bin, so the
                   energy is highly unreliable (high variance). For that reason,
                   we take the max with the previous energy so that at least 2 bins
                   are getting used. */
                for i in 0..IMIN(8, end) {
                    bandLogE3[i as usize] = MAXG(
                        bandLogE2[(c * nbEBands + i) as usize],
                        oldBandE[(c * nbEBands + i) as usize],
                    );
                }
            }
            let fbase = (c * nbEBands) as usize;
            follower[fbase] = bandLogE3[0];
            for i in 1..end {
                /* The last band to be at least 3 dB higher than the previous one
                   is the last we'll consider. Otherwise, we run into problems on
                   bandlimited signals. */
                if bandLogE3[i as usize] > bandLogE3[(i - 1) as usize] + GCONST(0.5f32) {
                    last = i;
                }
                follower[fbase + i as usize] = MING(
                    follower[fbase + (i - 1) as usize] + GCONST(1.5f32),
                    bandLogE3[i as usize],
                );
            }
            let mut i = last - 1;
            while i >= 0 {
                follower[fbase + i as usize] = MING(
                    follower[fbase + i as usize],
                    MING(follower[fbase + (i + 1) as usize] + GCONST(2.0f32), bandLogE3[i as usize]),
                );
                i -= 1;
            }

            /* Combine with a median filter to avoid dynalloc triggering unnecessarily.
               The "offset" value controls how conservative we are -- a higher offset
               reduces the impact of the median filter and makes dynalloc use more bits. */
            offset = GCONST(1.0f32);
            for i in 2..end - 2 {
                follower[fbase + i as usize] = MAXG(
                    follower[fbase + i as usize],
                    median_of_5(&bandLogE3[(i - 2) as usize..]) - offset,
                );
            }
            tmp = median_of_3(&bandLogE3[0..]) - offset;
            follower[fbase] = MAXG(follower[fbase], tmp);
            follower[fbase + 1] = MAXG(follower[fbase + 1], tmp);
            tmp = median_of_3(&bandLogE3[(end - 3) as usize..]) - offset;
            follower[fbase + (end - 2) as usize] = MAXG(follower[fbase + (end - 2) as usize], tmp);
            follower[fbase + (end - 1) as usize] = MAXG(follower[fbase + (end - 1) as usize], tmp);

            for i in 0..end {
                follower[fbase + i as usize] = MAXG(follower[fbase + i as usize], noise_floor[i as usize]);
            }
            c += 1;
            if c >= C {
                break;
            }
        }
        if C == 2 {
            for i in start..end {
                /* Consider 24 dB "cross-talk" */
                follower[(nbEBands + i) as usize] = MAXG(
                    follower[(nbEBands + i) as usize],
                    follower[i as usize] - GCONST(4.0f32),
                );
                follower[i as usize] = MAXG(
                    follower[i as usize],
                    follower[(nbEBands + i) as usize] - GCONST(4.0f32),
                );
                follower[i as usize] = HALF32(
                    MAXG(0.0f32, bandLogE[i as usize] - follower[i as usize])
                        + MAXG(
                            0.0f32,
                            bandLogE[(nbEBands + i) as usize] - follower[(nbEBands + i) as usize],
                        ),
                );
            }
        } else {
            for i in start..end {
                follower[i as usize] = MAXG(0.0f32, bandLogE[i as usize] - follower[i as usize]);
            }
        }
        for i in start..end {
            follower[i as usize] = MAXG(follower[i as usize], surround_dynalloc[i as usize]);
        }
        for i in start..end {
            importance[i as usize] = ((0.5f32 + 13.0f32 * celt_exp2_db(MING(follower[i as usize], GCONST(4.0f32)))) as f64)
                .floor() as i32;
        }
        /* For non-transient CBR/CVBR frames, halve the dynalloc contribution */
        if (vbr == 0 || constrained_vbr != 0) && isTransient == 0 {
            for i in start..end {
                follower[i as usize] = HALF32(follower[i as usize]);
            }
        }
        for i in start..end {
            if i < 8 {
                follower[i as usize] *= 2.0;
            }
            if i >= 12 {
                follower[i as usize] = HALF32(follower[i as usize]);
            }
        }
        /* Compensate for Opus' under-allocation on tones. */
        if toneishness > QCONST32(0.98f32, 29) {
            let freq_bin = ((0.5f64 + (tone_freq as f64) * 120.0f64 / PI).floor()) as i32;
            for i in start..end {
                if freq_bin >= eBands[i as usize] as i32 && freq_bin <= eBands[(i + 1) as usize] as i32 {
                    follower[i as usize] += GCONST(2.0f32);
                }
                if freq_bin >= eBands[i as usize] as i32 - 1
                    && freq_bin <= eBands[(i + 1) as usize] as i32 + 1
                {
                    follower[i as usize] += GCONST(1.0f32);
                }
                if freq_bin >= eBands[i as usize] as i32 - 2
                    && freq_bin <= eBands[(i + 1) as usize] as i32 + 2
                {
                    follower[i as usize] += GCONST(1.0f32);
                }
                if freq_bin >= eBands[i as usize] as i32 - 3
                    && freq_bin <= eBands[(i + 1) as usize] as i32 + 3
                {
                    follower[i as usize] += GCONST(0.5f32);
                }
            }
            if freq_bin >= eBands[end as usize] as i32 {
                follower[(end - 1) as usize] += GCONST(2.0f32);
                follower[(end - 2) as usize] += GCONST(1.0f32);
            }
        }
        /* `#ifndef DISABLE_FLOAT_API` */
        if analysis.valid != 0 {
            for i in start..IMIN(LEAK_BANDS, end) {
                follower[i as usize] =
                    follower[i as usize] + GCONST(1.0f32 / 64.0f32) * analysis.leak_boost[i as usize] as opus_val32;
            }
        }
        for i in start..end {
            let width;
            let boost;
            let boost_bits;

            follower[i as usize] = MING(follower[i as usize], GCONST(4.0f32));

            follower[i as usize] = SHR32(follower[i as usize], 8);
            width = C * ((eBands[(i + 1) as usize] as i32) - (eBands[i as usize] as i32)) << LM;
            if width < 6 {
                boost = SHR32(follower[i as usize], DB_SHIFT - 8) as i32;
                boost_bits = boost * width << BITRES;
            } else if width > 48 {
                boost = SHR32(follower[i as usize] * 8.0f32, DB_SHIFT - 8) as i32;
                boost_bits = (boost * width << BITRES) / 8;
            } else {
                boost = SHR32(follower[i as usize] * width as opus_val32 / 6.0f32, DB_SHIFT - 8) as i32;
                boost_bits = boost * 6 << BITRES;
            }
            /* For CBR and non-transient CVBR frames, limit dynalloc to 2/3 of the bits */
            if (vbr == 0 || (constrained_vbr != 0 && isTransient == 0))
                && (tot_boost + boost_bits) >> BITRES >> 3 > 2 * effectiveBytes / 3
            {
                let cap = (2 * effectiveBytes / 3) << BITRES << 3;
                offsets[i as usize] = cap - tot_boost;
                tot_boost = cap;
                break;
            } else {
                offsets[i as usize] = boost;
                tot_boost += boost_bits;
            }
        }
    } else {
        for i in start..end {
            importance[i as usize] = 13;
        }
    }
    *tot_boost_ = tot_boost;
    maxDepth
}

/* ------------------------------------------------------------------------- *
 * tone_lpc / tone_detect
 * ------------------------------------------------------------------------- */

/// `tone_lpc()` (`celt/celt_encoder.c:1305`).
fn tone_lpc(x: &[opus_val16], len: i32, delay: i32, lpc: &mut [opus_val32; 2]) -> i32 {
    let mut r00: opus_val32 = 0.0;
    let mut r01: opus_val32 = 0.0;
    let mut r11: opus_val32 = 0.0;
    let mut r02: opus_val32 = 0.0;
    let mut r12: opus_val32 = 0.0;
    let mut r22: opus_val32 = 0.0;
    let mut edges: opus_val32;
    celt_assert!(len > 2 * delay);
    /* Compute correlations as if using the forward prediction covariance method. */
    for i in 0..len - 2 * delay {
        r00 += MULT16_16(x[i as usize], x[i as usize]);
        r01 += MULT16_16(x[i as usize], x[(i + delay) as usize]);
        r02 += MULT16_16(x[i as usize], x[(i + 2 * delay) as usize]);
    }
    edges = 0.0;
    for i in 0..delay {
        edges += MULT16_16(x[(len + i - 2 * delay) as usize], x[(len + i - 2 * delay) as usize])
            - MULT16_16(x[i as usize], x[i as usize]);
    }
    r11 = r00 + edges;
    edges = 0.0;
    for i in 0..delay {
        edges += MULT16_16(x[(len + i - delay) as usize], x[(len + i - delay) as usize])
            - MULT16_16(x[(i + delay) as usize], x[(i + delay) as usize]);
    }
    r22 = r11 + edges;
    edges = 0.0;
    for i in 0..delay {
        edges += MULT16_16(x[(len + i - 2 * delay) as usize], x[(len + i - delay) as usize])
            - MULT16_16(x[i as usize], x[(i + delay) as usize]);
    }
    r12 = r01 + edges;
    /* Reverse and sum to get the backward contribution. */
    {
        let R00 = r00 + r22;
        let R01 = r01 + r12;
        let R11 = 2.0f32 * r11;
        let R02 = 2.0f32 * r02;
        let R12 = r12 + r01;
        let R22 = r00 + r22;
        r00 = R00;
        r01 = R01;
        r11 = R11;
        r02 = R02;
        r12 = R12;
        r22 = R22;
    }
    /* Solve A*x=b, where A=[r00, r01; r01, r11] and b=[r02; r12]. */
    let den = MULT32_32_Q31(r00, r11) - MULT32_32_Q31(r01, r01);
    /* `#ifdef FIXED_POINT` is not compiled. */
    if den < 0.001f32 * MULT32_32_Q31(r00, r11) {
        return 1;
    }
    let num1 = MULT32_32_Q31(r02, r11) - MULT32_32_Q31(r01, r12);
    if num1 >= den {
        lpc[1] = QCONST32(1.0f32, 29);
    } else if num1 <= -den {
        lpc[1] = -QCONST32(1.0f32, 29);
    } else {
        lpc[1] = frac_div32_q29(num1, den);
    }
    let num0 = MULT32_32_Q31(r00, r12) - MULT32_32_Q31(r02, r01);
    if HALF32(num0) >= den {
        lpc[0] = QCONST32(1.999999f32, 29);
    } else if HALF32(num0) <= -den {
        lpc[0] = -QCONST32(1.999999f32, 29);
    } else {
        lpc[0] = frac_div32_q29(num0, den);
    }
    0
}

/// `tone_detect()` (`celt/celt_encoder.c:1362`).
fn tone_detect(
    in_: &[celt_sig],
    CC: i32,
    N: i32,
    toneishness: &mut opus_val32,
    Fs: opus_int32,
) -> opus_val16 {
    let mut delay = 1i32;
    let mut fail;
    let mut lpc = [0.0f32 as opus_val32; 2];
    let freq;
    let mut x = vec![0.0f32 as opus_val16; N as usize];
    /* Shift by SIG_SHIFT+2 (+3 for stereo) to account for HF gain of the preemphasis filter. */
    if CC == 2 {
        for i in 0..N as usize {
            x[i] = PSHR32(ADD32(SHR32(in_[i], 1), SHR32(in_[i + N as usize], 1)), SIG_SHIFT + 2);
        }
    } else {
        for i in 0..N as usize {
            x[i] = PSHR32(in_[i], SIG_SHIFT + 2);
        }
    }
    /* `#ifdef FIXED_POINT` normalize_tone_input is not compiled. */
    fail = tone_lpc(&x, N, delay, &mut lpc);
    /* If our LPC filter resonates too close to DC, retry the analysis with down-sampling. */
    while delay <= Fs / 3000 && (fail != 0 || (lpc[0] > QCONST32(1.0f32, 29) && lpc[1] < 0.0)) {
        delay *= 2;
        fail = tone_lpc(&x, N, delay, &mut lpc);
    }
    /* Check that our filter has complex roots. */
    if fail == 0
        && MULT32_32_Q31(lpc[0], lpc[0]) + MULT32_32_Q31(QCONST32(3.999999f32, 29), lpc[1]) < 0.0
    {
        /* Squared radius of the poles. */
        *toneishness = -lpc[1];
        /* `#ifdef FIXED_POINT` acos_approx is not compiled: the C calls the
           double `acos()`. */
        freq = (((0.5f32 * lpc[0]) as f64).acos() / delay as f64) as opus_val16;
    } else {
        let freq_: opus_val16 = -1.0;
        *toneishness = 0.0;
        freq = freq_;
    }
    freq
}

/* ------------------------------------------------------------------------- *
 * run_prefilter
 * ------------------------------------------------------------------------- */

/// `run_prefilter()` (`celt/celt_encoder.c:1404`).
///
/// `st->prefilter_period` is only *read* before the comb-filter pass (the C
/// mutation `st->prefilter_period = IMAX(..., COMBFILTER_MINPERIOD)` does not
/// survive the caller's later `st->prefilter_period = pitch_index`), so the
/// clamp is done locally here and the field itself is not touched.
#[allow(clippy::too_many_arguments)]
fn run_prefilter(
    mode: &CELTMode,
    arch: i32,
    in_: &mut [celt_sig],
    prefilter_mem: &mut [celt_sig],
    in_mem: &mut [celt_sig],
    CC: i32,
    N: i32,
    prefilter_tapset: i32,
    pitch: &mut i32,
    gain: &mut opus_val16,
    qgain: &mut i32,
    enabled: i32,
    complexity: i32,
    tf_estimate: opus_val16,
    nbAvailableBytes: i32,
    analysis: &AnalysisInfo,
    tone_freq_in: opus_val16,
    toneishness: opus_val32,
    prefilter_period_in: i32,
    prefilter_gain_in: opus_val16,
    loss_rate: i32,
) -> i32 {
    let max_period = COMBFILTER_MAXPERIOD;
    let min_period = COMBFILTER_MINPERIOD;
    let overlap = mode.overlap;
    let mut _pre = vec![0.0f32 as celt_sig; (CC * (N + max_period)) as usize];
    let pre_len = (N + max_period) as usize;

    for c in 0..CC as usize {
        _pre[c * pre_len..c * pre_len + max_period as usize]
            .copy_from_slice(&prefilter_mem[c * max_period as usize..c * max_period as usize + max_period as usize]);
        let src = c * (N + overlap) as usize + overlap as usize;
        _pre[c * pre_len + max_period as usize..c * pre_len + max_period as usize + N as usize]
            .copy_from_slice(&in_[src..src + N as usize]);
    }

    let mut pitch_index: i32 = 0;
    let mut gain1: opus_val16;
    let mut tone_freq = tone_freq_in;

    /* If we detect that the signal is dominated by a single tone, don't rely on the standard pitch
       estimator, as it can become unreliable. */
    if enabled != 0 && toneishness > QCONST32(0.99f32, 29) {
        let mut multiple = 1i32;
        /* Using aliased version of the postfilter above 24 kHz.
           First value is purposely slightly above pi to avoid triggering for Fs=48kHz. */
        if tone_freq >= QCONST16(3.1416f32, 13) {
            tone_freq = QCONST16(3.141593f32, 13) - tone_freq;
        }
        /* If the pitch is too high for our post-filter, apply pitch doubling until
           we can get something that fits (not ideal, but better than nothing). */
        while tone_freq >= multiple as opus_val16 * QCONST16(0.39f32, 13) {
            multiple += 1;
        }
        if tone_freq > QCONST16(0.006148f32, 13) {
            pitch_index = IMIN(
                (0.5f64 + 2.0f64 * PI * multiple as f64 / tone_freq as f64).floor() as i32,
                COMBFILTER_MAXPERIOD - 2,
            );
        } else {
            /* If the pitch is too low, using a very high pitch will actually give us an improvement
               due to the DC component of the filter that will be close to our tone. Again, not ideal,
               but if we only have a single tone, it's better than nothing. */
            pitch_index = COMBFILTER_MINPERIOD;
        }
        gain1 = QCONST16(0.75f32, 15);
    } else if enabled != 0 && complexity >= 5 {
        let plen = ((max_period + N) >> 1) as usize;
        let mut pitch_buf = vec![0.0f32 as opus_val16; plen];

        let views: [&[celt_sig]; 2] = [&_pre[0..], &_pre[pre_len..]];
        pitch_downsample(
            &views[..CC as usize],
            &mut pitch_buf,
            (max_period + N) >> 1,
            CC,
            2,
            arch,
        );
        /* Don't search for the fir last 1.5 octave of the range because
           there's too many false-positives due to short-term correlation */
        pitch_search(
            &pitch_buf[(max_period >> 1) as usize..],
            &pitch_buf,
            N,
            max_period - 3 * min_period,
            &mut pitch_index,
            arch,
        );
        pitch_index = max_period - pitch_index;

        gain1 = remove_doubling(
            &pitch_buf,
            max_period,
            min_period,
            N,
            &mut pitch_index,
            prefilter_period_in,
            prefilter_gain_in,
            arch,
        );
        if pitch_index > max_period - 2 {
            pitch_index = max_period - 2;
        }
        gain1 = MULT16_16_Q15(QCONST16(0.7f32, 15), gain1);
        if loss_rate > 2 {
            gain1 = HALF32(gain1);
        }
        if loss_rate > 4 {
            gain1 = HALF32(gain1);
        }
        if loss_rate > 8 {
            gain1 = 0.0;
        }
    } else {
        gain1 = 0.0;
        pitch_index = COMBFILTER_MINPERIOD;
    }
    /* `#ifndef DISABLE_FLOAT_API` */
    if analysis.valid != 0 {
        gain1 = gain1 * analysis.max_pitch_ratio;
    }
    /* Gain threshold for enabling the prefilter/postfilter */
    let mut pf_threshold = QCONST16(0.2f32, 15);

    /* Adjusting the threshold based on rate and continuity */
    if (pitch_index - prefilter_period_in).abs() * 10 > pitch_index {
        pf_threshold += QCONST16(0.2f32, 15);
        /* Completely disable the prefilter on strong transients without continuity. */
        if tf_estimate > QCONST16(0.98f32, 14) {
            gain1 = 0.0;
        }
    }
    if nbAvailableBytes < 25 {
        pf_threshold += QCONST16(0.1f32, 15);
    }
    if nbAvailableBytes < 35 {
        pf_threshold += QCONST16(0.1f32, 15);
    }
    if prefilter_gain_in > QCONST16(0.4f32, 15) {
        pf_threshold -= QCONST16(0.1f32, 15);
    }
    if prefilter_gain_in > QCONST16(0.55f32, 15) {
        pf_threshold -= QCONST16(0.1f32, 15);
    }

    /* Hard threshold at 0.2 */
    pf_threshold = MAX16(pf_threshold, QCONST16(0.2f32, 15));
    let mut pf_on: i32;
    let mut qg: i32;
    if gain1 < pf_threshold {
        gain1 = 0.0;
        pf_on = 0;
        qg = 0;
    } else {
        /*This block is not gated by a total bits check only because
          of the nbAvailableBytes check above.*/
        if ABS16(gain1 - prefilter_gain_in) < QCONST16(0.1f32, 15) {
            gain1 = prefilter_gain_in;
        }

        /* `#ifndef FIXED_POINT` */
        qg = ((0.5f32 + gain1 * 32.0f32 / 3.0f32) as f64).floor() as i32 - 1;
        qg = IMAX(0, IMIN(7, qg));
        gain1 = QCONST16(0.09375f32, 15) * (qg + 1) as opus_val16;
        pf_on = 1;
    }

    let mut before = [0.0f32 as opus_val32; 2];
    let mut after = [0.0f32 as opus_val32; 2];
    let mut cancel_pitch = 0i32;
    let prefilter_period = IMAX(prefilter_period_in, COMBFILTER_MINPERIOD);
    for c in 0..CC as usize {
        let offset = mode.shortMdctSize - overlap;
        let inbase = c * (N + overlap) as usize;
        in_[inbase..inbase + overlap as usize]
            .copy_from_slice(&in_mem[c * overlap as usize..c * overlap as usize + overlap as usize]);
        for i in 0..N as usize {
            before[c] += ABS32(SHR32(in_[inbase + overlap as usize + i], 12));
        }
        if offset != 0 {
            let h = IMAX(IMAX(prefilter_period, prefilter_period), COMBFILTER_MINPERIOD) + 2;
            let ystart = inbase + overlap as usize;
            let xstart = (c * pre_len + max_period as usize) - h as usize;
            comb_filter(
                &mut in_[ystart..],
                &_pre[xstart..],
                prefilter_period,
                prefilter_period,
                offset,
                -prefilter_gain_in,
                -prefilter_gain_in,
                prefilter_tapset,
                prefilter_tapset,
                &[],
                0,
                arch,
            );
        }

        let h = IMAX(IMAX(prefilter_period, pitch_index), COMBFILTER_MINPERIOD) + 2;
        let ystart = inbase + overlap as usize + offset as usize;
        let xstart = c * pre_len + max_period as usize + offset as usize - h as usize;
        comb_filter(
            &mut in_[ystart..],
            &_pre[xstart..],
            prefilter_period,
            pitch_index,
            N - offset,
            -prefilter_gain_in,
            -gain1,
            prefilter_tapset,
            prefilter_tapset,
            &mode.window,
            overlap,
            arch,
        );
        for i in 0..N as usize {
            after[c] += ABS32(SHR32(in_[inbase + overlap as usize + i], 12));
        }
    }

    if CC == 2 {
        let mut thresh = [0.0f32 as opus_val16; 2];
        thresh[0] = MULT16_32_Q15(
            MULT16_16_Q15(QCONST16(0.25f32, 15), gain1),
            before[0],
        ) + MULT16_32_Q15(QCONST16(0.01f32, 15), before[1]);
        thresh[1] = MULT16_32_Q15(
            MULT16_16_Q15(QCONST16(0.25f32, 15), gain1),
            before[1],
        ) + MULT16_32_Q15(QCONST16(0.01f32, 15), before[0]);
        /* Don't use the filter if one channel gets significantly worse. */
        if after[0] - before[0] > thresh[0] || after[1] - before[1] > thresh[1] {
            cancel_pitch = 1;
        }
        /* Use the filter only if at least one channel gets significantly better. */
        if before[0] - after[0] < thresh[0] && before[1] - after[1] < thresh[1] {
            cancel_pitch = 1;
        }
    } else {
        /* Check that the mono channel actually got better. */
        if after[0] > before[0] {
            cancel_pitch = 1;
        }
    }
    /* If needed, revert to a gain of zero. */
    if cancel_pitch != 0 {
        for c in 0..CC as usize {
            let offset = mode.shortMdctSize - overlap;
            let inbase = c * (N + overlap) as usize;
            in_[inbase + overlap as usize..inbase + overlap as usize + N as usize].copy_from_slice(
                &_pre[c * pre_len + max_period as usize
                    ..c * pre_len + max_period as usize + N as usize],
            );
            let h = IMAX(IMAX(prefilter_period, pitch_index), COMBFILTER_MINPERIOD) + 2;
            let ystart = inbase + overlap as usize + offset as usize;
            let xstart = c * pre_len + max_period as usize + offset as usize - h as usize;
            comb_filter(
                &mut in_[ystart..],
                &_pre[xstart..],
                prefilter_period,
                pitch_index,
                overlap,
                -prefilter_gain_in,
                -0.0f32,
                prefilter_tapset,
                prefilter_tapset,
                &mode.window,
                overlap,
                arch,
            );
        }
        gain1 = 0.0;
        pf_on = 0;
        qg = 0;
    }

    for c in 0..CC as usize {
        let inbase = c * (N + overlap) as usize;
        in_mem[c * overlap as usize..c * overlap as usize + overlap as usize]
            .copy_from_slice(&in_[inbase + N as usize..inbase + N as usize + overlap as usize]);

        if N > max_period {
            prefilter_mem[c * max_period as usize..c * max_period as usize + max_period as usize]
                .copy_from_slice(&_pre[c * pre_len + N as usize..c * pre_len + N as usize + max_period as usize]);
        } else {
            let base = c * max_period as usize;
            prefilter_mem.copy_within(base + N as usize..base + max_period as usize, base);
            prefilter_mem[base + max_period as usize - N as usize..base + max_period as usize]
                .copy_from_slice(&_pre[c * pre_len + max_period as usize..c * pre_len + max_period as usize + N as usize]);
        }
    }

    *gain = gain1;
    *pitch = pitch_index;
    *qgain = qg;
    pf_on
}

/* ------------------------------------------------------------------------- *
 * compute_vbr
 * ------------------------------------------------------------------------- */

/// `compute_vbr()` (`celt/celt_encoder.c:1604`).
#[allow(clippy::too_many_arguments)]
fn compute_vbr(
    mode: &CELTMode,
    analysis: &AnalysisInfo,
    base_target: opus_int32,
    LM: i32,
    bitrate: opus_int32,
    lastCodedBands: i32,
    C: i32,
    intensity: i32,
    constrained_vbr: i32,
    stereo_saving_in: opus_val16,
    tot_boost: i32,
    tf_estimate: opus_val16,
    pitch_change: i32,
    maxDepth: celt_glog,
    lfe: i32,
    has_surround_mask: i32,
    surround_masking: celt_glog,
    temporal_vbr: celt_glog,
) -> opus_int32 {
    /* The target rate in 8th bits per frame */
    let mut target: opus_int32;
    let coded_bins;
    let nbEBands: i32;
    let eBands: &[opus_int16];

    nbEBands = mode.nbEBands;
    eBands = &mode.eBands;

    let coded_bands = if lastCodedBands != 0 { lastCodedBands } else { nbEBands };
    coded_bins = ((eBands[coded_bands as usize] as i32) << LM)
        + if C == 2 {
            (eBands[IMIN(intensity, coded_bands) as usize] as i32) << LM
        } else {
            0
        };

    target = base_target;

    /* `#ifndef DISABLE_FLOAT_API` */
    if analysis.valid != 0 && analysis.activity < 0.4 {
        target -= ((coded_bins << BITRES) as opus_val32 * (0.4f32 - analysis.activity)) as opus_int32;
    }
    /* Stereo savings */
    if C == 2 {
        let coded_stereo_bands;
        let coded_stereo_dof;
        let max_frac;
        coded_stereo_bands = IMIN(intensity, coded_bands);
        coded_stereo_dof = ((eBands[coded_stereo_bands as usize] as i32) << LM) - coded_stereo_bands;
        /* Maximum fraction of the bits we can save if the signal is mono. */
        max_frac = DIV32_16(
            MULT16_16(QCONST16(0.8f32, 15), coded_stereo_dof as opus_val32),
            coded_bins as opus_val16,
        );
        let stereo_saving = MIN16(stereo_saving_in, QCONST16(1.0f32, 8));
        target -= MIN32(
            MULT16_32_Q15(max_frac, target as opus_val32),
            SHR32(
                MULT16_16(
                    stereo_saving - QCONST16(0.1f32, 8),
                    (coded_stereo_dof << BITRES) as opus_val32,
                ),
                8,
            ),
        ) as opus_int32;
    }
    /* Boost the rate according to dynalloc (minus the dynalloc average for calibration). */
    target += tot_boost - (19 << LM);
    /* Apply transient boost, compensating for average boost. */
    let tf_calibration = QCONST16(0.044f32, 14);
    target += SHL32(MULT16_32_Q15(tf_estimate - tf_calibration, target as opus_val32), 1) as opus_int32;

    /* `#ifndef DISABLE_FLOAT_API` */
    /* Apply tonality boost */
    if analysis.valid != 0 && lfe == 0 {
        let tonal_target;
        let tonal;

        /* Tonality boost (compensating for the average). */
        tonal = MAX16(0.0f32, analysis.tonality - 0.15f32) - 0.12f32;
        tonal_target = target + ((coded_bins << BITRES) as opus_val32 * 1.2f32 * tonal) as opus_int32;
        let mut tonal_target = tonal_target;
        if pitch_change != 0 {
            tonal_target += ((coded_bins << BITRES) as opus_val32 * 0.8f32) as opus_int32;
        }
        target = tonal_target;
    }

    if has_surround_mask != 0 && lfe == 0 {
        let surround_target = target
            + SHR32(
                MULT16_16(
                    SHR32(surround_masking, DB_SHIFT - 10),
                    (coded_bins << BITRES) as opus_val32,
                ),
                10,
            ) as opus_int32;
        target = IMAX(target / 4, surround_target);
    }

    {
        let floor_depth;
        let bins;
        bins = (eBands[(nbEBands - 2) as usize] as i32) << LM;
        /*floor_depth = SHR32(MULT16_16((C*bins<<BITRES),celt_log2(SHL32(MAX16(1,sample_max),13))), DB_SHIFT);*/
        floor_depth = SHR32(
            MULT16_32_Q15((C * bins << BITRES) as opus_val32, maxDepth),
            DB_SHIFT - 15,
        ) as opus_int32;
        let floor_depth = IMAX(floor_depth, target >> 2);
        target = IMIN(target, floor_depth);
    }

    /* Make VBR less aggressive for constrained VBR because we can't keep a higher bitrate
       for long. Needs tuning. */
    if (has_surround_mask == 0 || lfe != 0) && constrained_vbr != 0 {
        target = base_target
            + MULT16_32_Q15(QCONST16(0.67f32, 15), (target - base_target) as opus_val32) as opus_int32;
    }

    if has_surround_mask == 0 && tf_estimate < QCONST16(0.2f32, 14) {
        let amount;
        let tvbr_factor;
        amount = MULT16_16_Q15(
            QCONST16(0.0000031f32, 30),
            IMAX(0, IMIN(32000, 96000 - bitrate)) as opus_val32,
        );
        tvbr_factor = SHR32(MULT16_16(SHR32(temporal_vbr, DB_SHIFT - 10), amount), 10);
        target += MULT16_32_Q15(tvbr_factor, target as opus_val32) as opus_int32;
    }

    /* Don't allow more than doubling the rate */
    target = IMIN(2 * base_target, target);

    target
}

/* ------------------------------------------------------------------------- *
 * celt_encode_with_ec
 * ------------------------------------------------------------------------- */

/// `celt_encode_with_ec()` (`celt/celt_encoder.c:1725`).
///
/// `enc == None` is the C `enc == NULL`: a local range coder is created over
/// `compressed` and the finished packet is copied back into `compressed` on
/// return.  With `Some(enc)` the caller owns the coder and the bytes are read
/// from it instead (this is how `opus_encoder.c` uses the function).
///
/// `ENABLE_QEXT` is undefined, so `qext_bytes` is always 0 and every QEXT
/// block of the C source is absent.
#[allow(clippy::too_many_arguments)]
pub fn celt_encode_with_ec(
    st: &mut OpusCustomEncoder,
    pcm: &[opus_res],
    frame_size: i32,
    compressed: &mut [u8],
    nbCompressedBytes: i32,
    mut enc_in: Option<&mut ec_ctx>,
) -> i32 {
    let mut c: i32;
    let N: i32;
    let mut bits: opus_int32;
    let mut shortBlocks = 0i32;
    let mut isTransient = 0i32;
    let CC = st.channels;
    let C = st.stream_channels;
    let mut LM: i32;
    let M: i32;
    let mut tf_select: i32 = 0;
    let mut nbFilledBytes: i32;
    let mut nbAvailableBytes: i32;
    let mut min_allowed: opus_int32;
    let mut start: i32;
    let mut end: i32;
    let mut effEnd: i32;
    let mut codedBands: i32;
    let mut alloc_trim: i32;
    let mut pitch_index: i32 = COMBFILTER_MINPERIOD;
    let mut gain1: opus_val16 = 0.0;
    let mut dual_stereo = 0i32;
    let mut effectiveBytes: i32;
    let mut dynalloc_logp: i32;
    let mut vbr_rate: opus_int32;
    let mut total_bits: opus_int32;
    let mut total_boost: opus_int32;
    let mut balance: opus_int32 = 0;
    let mut tell: opus_int32;
    let mut tell0_frac: opus_int32;
    let mut prefilter_tapset = 0i32;
    let mut pf_on: i32;
    let mut anti_collapse_rsv: i32;
    let mut silence = 0i32;
    let mut tf_chan = 0i32;
    let mut tf_estimate: opus_val16;
    let mut pitch_change = 0i32;
    let mut tot_boost: opus_int32 = 0;
    let mut sample_max: opus_val32;
    let maxDepth: celt_glog;
    let mut secondMdct: i32;
    let mut signalBandwidth: i32;
    let mut transient_got_disabled = 0i32;
    let mut surround_masking: celt_glog = 0.0;
    let mut temporal_vbr: celt_glog = 0.0;
    let mut surround_trim: celt_glog = 0.0;
    let mut equiv_rate: opus_int32;
    let hybrid: i32;
    let mut weak_transient = 0i32;
    let enable_tf_analysis: i32;
    let mut tone_freq: opus_val16 = -1.0;
    let mut toneishness: opus_val32 = 0.0;
    let mut surround_dynalloc: Vec<celt_glog>;

    let packet_size_cap: i32 = 1275;
    let mut frame_size = frame_size;
    let mut nbCompressedBytes = nbCompressedBytes;

    let mode: &CELTMode = &st.mode;
    let nbEBands = mode.nbEBands;
    let overlap = mode.overlap;
    let eBands: &[opus_int16] = &mode.eBands;
    start = st.start;
    end = st.end;
    hybrid = (start != 0) as i32;
    tf_estimate = 0.0;
    if nbCompressedBytes < 2 || pcm.is_empty() {
        return OPUS_BAD_ARG;
    }

    frame_size *= st.upsample;
    LM = 0;
    while LM <= mode.maxLM {
        if (mode.shortMdctSize << LM) == frame_size {
            break;
        }
        LM += 1;
    }
    if LM > mode.maxLM {
        return OPUS_BAD_ARG;
    }
    M = 1 << LM;
    N = M * mode.shortMdctSize;

    /* `#ifdef ENABLE_QEXT` is not compiled: packet_size_cap stays 1275. */

    /* `prefilter_mem`, `oldBandE`, `oldLogE`, `oldLogE2` and `energyError` are
       the fields of `st` directly. */

    let use_local = enc_in.is_none();
    if use_local {
        tell0_frac = 1;
        tell = 1;
        nbFilledBytes = 0;
    } else {
        let e = enc_in.as_ref().unwrap();
        tell0_frac = ec_tell_frac(e) as opus_int32;
        tell = e.ec_tell();
        nbFilledBytes = (tell + 4) >> 3;
    }

    /* `#if defined(CUSTOM_MODES) || defined(ENABLE_OPUS_CUSTOM_API)` */
    let mut signal_offset: usize = 0;
    if st.signalling != 0 && use_local {
        let tmp = (mode.effEBands - end) >> 1;
        st.end = IMAX(1, mode.effEBands - tmp);
        end = st.end;
        let mut b0 = ((tmp << 5) | (LM << 3) | (((C == 2) as i32) << 2)) as u8;
        /* Convert "standard mode" to Opus header */
        /* `#ifndef ENABLE_QEXT` */
        if mode.Fs == 48000 && mode.shortMdctSize == 120 {
            let c0 = toOpus(b0);
            if c0 < 0 {
                return OPUS_BAD_ARG;
            }
            b0 = c0 as u8;
        }
        compressed[0] = b0;
        signal_offset = 1;
        nbCompressedBytes -= 1;
    }

    /* Can't produce more than 1275 output bytes for the main payload. */
    nbCompressedBytes = IMIN(nbCompressedBytes, packet_size_cap);

    if st.vbr != 0 && st.bitrate != OPUS_BITRATE_MAX {
        vbr_rate = bitrate_to_bits(st.bitrate, mode.Fs, frame_size) << BITRES;
        /* `#if defined(CUSTOM_MODES) || ...` */
        if st.signalling != 0 {
            vbr_rate -= 8 << BITRES;
        }
        effectiveBytes = vbr_rate >> (3 + BITRES);
    } else {
        let mut tmp: opus_int32;
        vbr_rate = 0;
        tmp = st.bitrate * frame_size;
        if tell > 1 {
            tmp += tell * mode.Fs;
        }
        if st.bitrate != OPUS_BITRATE_MAX {
            nbCompressedBytes = IMAX(
                2,
                IMIN(
                    nbCompressedBytes,
                    (tmp + 4 * mode.Fs) / (8 * mode.Fs) - (st.signalling != 0) as i32,
                ),
            );
            if !use_local {
                if let Some(e) = enc_in.as_deref_mut() {
                    ec_enc_shrink(e, nbCompressedBytes as opus_uint32);
                }
            }
        }
        effectiveBytes = nbCompressedBytes - nbFilledBytes;
    }
    nbAvailableBytes = nbCompressedBytes - nbFilledBytes;
    equiv_rate = (nbCompressedBytes * 8 * 50 << (3 - LM)) - (40 * C + 20) * ((400 >> LM) - 50);
    if st.bitrate != OPUS_BITRATE_MAX {
        equiv_rate = IMIN(equiv_rate, st.bitrate - (40 * C + 20) * ((400 >> LM) - 50));
    }

    let mut _enc_storage = ec_enc_init(if use_local { nbCompressedBytes as opus_uint32 } else { 0 });
    let enc: &mut ec_ctx = match enc_in {
        Some(e) => e,
        None => &mut _enc_storage,
    };

    if vbr_rate > 0 {
        /* Computes the max bit-rate allowed in VBR mode to avoid violating the
            target rate and buffering. */
        if st.constrained_vbr != 0 {
            let vbr_bound: opus_int32;
            let max_allowed: opus_int32;
            vbr_bound = vbr_rate;
            max_allowed = IMIN(
                IMAX(
                    if tell == 1 { 2 } else { 0 },
                    (vbr_rate + vbr_bound - st.vbr_reservoir) >> (BITRES + 3),
                ),
                nbAvailableBytes,
            );
            if max_allowed < nbAvailableBytes {
                nbCompressedBytes = nbFilledBytes + max_allowed;
                nbAvailableBytes = max_allowed;
                ec_enc_shrink(enc, nbCompressedBytes as opus_uint32);
            }
        }
    }
    total_bits = nbCompressedBytes * 8;

    effEnd = end;
    if effEnd > mode.effEBands {
        effEnd = mode.effEBands;
    }

    let mut in_ = vec![0.0f32 as celt_sig; (CC * (N + overlap)) as usize];

    sample_max = MAX32(
        st.overlap_max,
        celt_maxabs_res(pcm, C * (N - overlap) / st.upsample),
    );
    st.overlap_max = celt_maxabs_res(
        &pcm[(C * (N - overlap) / st.upsample) as usize..],
        C * overlap / st.upsample,
    );
    sample_max = MAX32(sample_max, st.overlap_max);
    /* `#ifdef FIXED_POINT` is not compiled. */
    silence = (sample_max <= (1.0f32 / ((1i32 << st.lsb_depth) as opus_val32))) as i32;
    if tell == 1 {
        ec_enc_bit_logp(enc, silence, 15);
    } else {
        silence = 0;
    }
    if silence != 0 {
        /* In VBR mode there is no need to send more than the minimum. */
        if vbr_rate > 0 {
            nbCompressedBytes = IMIN(nbCompressedBytes, nbFilledBytes + 2);
            effectiveBytes = nbCompressedBytes;
            total_bits = nbCompressedBytes * 8;
            nbAvailableBytes = 2;
            ec_enc_shrink(enc, nbCompressedBytes as opus_uint32);
        }
        /* Pretend we've filled all the remaining bits with zeros
              (that's what the initialiser did anyway) */
        tell = nbCompressedBytes * 8;
        let tell_now = enc.ec_tell();
        enc.nbits_total += tell - tell_now;
    }
    c = 0;
    loop {
        /* `#ifdef FIXED_POINT` is not compiled. */
        let need_clip = (st.clip != 0 && sample_max > 65536.0f32) as i32;
        let inbase = c * (N + overlap);
        celt_preemphasis(
            &pcm[c as usize..],
            &mut in_[(inbase + overlap) as usize..],
            N,
            CC,
            st.upsample,
            &mode.preemph,
            &mut st.preemph_memE[c as usize],
            need_clip,
        );
        let src = ((1 + c) * COMBFILTER_MAXPERIOD - overlap) as usize;
        in_[inbase as usize..(inbase + overlap) as usize]
            .copy_from_slice(&st.prefilter_mem[src..src + overlap as usize]);
        c += 1;
        if c >= CC {
            break;
        }
    }

    tone_freq = tone_detect(&in_, CC, N + overlap, &mut toneishness, mode.Fs);
    isTransient = 0;
    shortBlocks = 0;
    if st.complexity >= 1 && st.lfe == 0 {
        /* Reduces the likelihood of energy instability on fricatives at low bitrate
           in hybrid mode. */
        let allow_weak_transients =
            (hybrid != 0 && effectiveBytes < 15 && st.silk_info.signalType != 2) as i32;
        isTransient = transient_analysis(
            &in_,
            N + overlap,
            CC,
            &mut tf_estimate,
            &mut tf_chan,
            allow_weak_transients,
            &mut weak_transient,
            tone_freq,
            toneishness,
        );
    }
    toneishness = MIN32(
        toneishness,
        QCONST32(1.0f32, 29) - SHL32(tf_estimate, 15),
    );
    /* Find pitch period and gain */
    {
        let enabled: i32;
        let mut qg: i32 = 0;
        enabled = (((st.lfe != 0 && nbAvailableBytes > 3) || nbAvailableBytes > 12 * C)
            && hybrid == 0
            && silence == 0
            && tell + 16 <= total_bits
            && st.disable_pf == 0) as i32;

        prefilter_tapset = st.tapset_decision;
        pf_on = run_prefilter(
            mode,
            st.arch,
            &mut in_,
            &mut st.prefilter_mem,
            &mut st.in_mem,
            CC,
            N,
            prefilter_tapset,
            &mut pitch_index,
            &mut gain1,
            &mut qg,
            enabled,
            st.complexity,
            tf_estimate,
            nbAvailableBytes,
            &st.analysis,
            tone_freq,
            toneishness,
            st.prefilter_period,
            st.prefilter_gain,
            st.loss_rate,
        );
        if (gain1 > QCONST16(0.4f32, 15) || st.prefilter_gain > QCONST16(0.4f32, 15))
            && (st.analysis.valid == 0 || st.analysis.tonality > 0.3)
            && ((pitch_index as f64) > 1.26f64 * st.prefilter_period as f64
                || (pitch_index as f64) < 0.79f64 * st.prefilter_period as f64)
        {
            pitch_change = 1;
        }
        if pf_on == 0 {
            if hybrid == 0 && tell + 16 <= total_bits {
                ec_enc_bit_logp(enc, 0, 1);
            }
        } else {
            /*This block is not gated by a total bits check only because
              of the nbAvailableBytes check above.*/
            let octave: i32;
            ec_enc_bit_logp(enc, 1, 1);
            pitch_index += 1;
            octave = EC_ILOG(pitch_index as opus_uint32) - 5;
            ec_enc_uint(enc, octave as opus_uint32, 6);
            ec_enc_bits(
                enc,
                (pitch_index - (16 << octave)) as opus_uint32,
                (4 + octave) as u32,
            );
            pitch_index -= 1;
            ec_enc_bits(enc, qg as opus_uint32, 3);
            ec_enc_icdf(enc, prefilter_tapset, &tapset_icdf, 2);
        }
    }
    if LM > 0 && enc.ec_tell() + 3 <= total_bits {
        if isTransient != 0 {
            shortBlocks = M;
        }
    } else {
        isTransient = 0;
        transient_got_disabled = 1;
    }

    let mut freq = vec![0.0f32 as celt_sig; (CC * N) as usize]; /* Interleaved signal MDCTs */
    let mut bandE = vec![0.0f32 as celt_ener; (nbEBands * CC) as usize];
    let mut bandLogE = vec![0.0f32 as celt_glog; (nbEBands * CC) as usize];

    secondMdct = (shortBlocks != 0 && st.complexity >= 8) as i32;
    let mut bandLogE2 = vec![0.0f32 as celt_glog; (C * nbEBands) as usize];
    if secondMdct != 0 {
        compute_mdcts(mode, 0, &in_, &mut freq, C, CC, LM, st.upsample, st.arch);
        compute_band_energies(mode, &freq, &mut bandE, effEnd, C, LM, st.arch);
        amp2Log2(mode, effEnd, end, &bandE, &mut bandLogE2, C);
        for c in 0..C {
            for i in 0..end {
                bandLogE2[(nbEBands * c + i) as usize] += HALF32(SHL32(LM as opus_val32, DB_SHIFT));
            }
        }
    }

    compute_mdcts(mode, shortBlocks, &in_, &mut freq, C, CC, LM, st.upsample, st.arch);
    /* This should catch any NaN in the CELT input. */
    celt_assert!(!celt_isnan(freq[0]) && (C == 1 || !celt_isnan(freq[N as usize])));
    if CC == 2 && C == 1 {
        tf_chan = 0;
    }
    compute_band_energies(mode, &freq, &mut bandE, effEnd, C, LM, st.arch);

    if st.lfe != 0 {
        for i in 2..end {
            bandE[i as usize] = IMIN(
                bandE[i as usize],
                MULT16_32_Q15(QCONST16(1e-4f32, 15), bandE[0]),
            );
            bandE[i as usize] = MAX32(bandE[i as usize], EPSILON);
        }
    }
    amp2Log2(mode, effEnd, end, &bandE, &mut bandLogE, C);

    surround_dynalloc = vec![0.0f32 as celt_glog; (C * nbEBands) as usize];
    for v in surround_dynalloc[..end as usize].iter_mut() {
        *v = 0.0;
    }
    /* This computes how much masking takes place between surround channels */
    if hybrid == 0 && st.energy_mask.is_some() && st.lfe == 0 {
        let mask_end: i32;
        let mut midband: i32;
        let mut count_dynalloc: i32;
        let mut mask_avg: opus_val32 = 0.0;
        let mut diff: opus_val32 = 0.0;
        let mut count = 0i32;
        mask_end = IMAX(2, st.lastCodedBands);
        for c in 0..C {
            for i in 0..mask_end {
                let mut mask: celt_glog;
                let mask16: opus_val16;
                let em = st.energy_mask.as_ref().unwrap();
                mask = MAXG(
                    MING(em[(nbEBands * c + i) as usize], GCONST(0.25f32)),
                    -GCONST(2.0f32),
                );
                if mask > 0.0 {
                    mask = HALF32(mask);
                }
                mask16 = SHR32(mask, DB_SHIFT - 10);
                mask_avg += MULT16_16(
                    mask16,
                    (eBands[(i + 1) as usize] as i32 - eBands[i as usize] as i32) as opus_val32,
                );
                count += eBands[(i + 1) as usize] as i32 - eBands[i as usize] as i32;
                diff += MULT16_16(mask16, (1 + 2 * i - mask_end) as opus_val32);
            }
        }
        celt_assert!(count > 0);
        mask_avg = SHL32(DIV32_16(mask_avg, count as opus_val16), DB_SHIFT - 10);
        mask_avg += GCONST(0.2f32);
        diff = SHL32(
            diff * 6.0f32 / (C * (mask_end - 1) * (mask_end + 1) * mask_end) as opus_val32,
            DB_SHIFT - 10,
        );
        /* Again, being conservative */
        diff = HALF32(diff);
        diff = MAX32(MIN32(diff, GCONST(0.031f32)), -GCONST(0.031f32));
        /* Find the band that's in the middle of the coded spectrum */
        midband = 0;
        while (eBands[(midband + 1) as usize] as i32) < (eBands[mask_end as usize] as i32) / 2 {
            midband += 1;
        }
        count_dynalloc = 0;
        for i in 0..mask_end {
            let lin: opus_val32;
            let mut unmask: celt_glog;
            let em = st.energy_mask.as_ref().unwrap();
            lin = mask_avg + diff * (i - midband) as opus_val32;
            if C == 2 {
                unmask = MAXG(em[i as usize], em[(nbEBands + i) as usize]);
            } else {
                unmask = em[i as usize];
            }
            unmask = MING(unmask, GCONST(0.0f32));
            unmask -= lin;
            if unmask > GCONST(0.25f32) {
                surround_dynalloc[i as usize] = unmask - GCONST(0.25f32);
                count_dynalloc += 1;
            }
        }
        if count_dynalloc >= 3 {
            /* If we need dynalloc in many bands, it's probably because our
               initial masking rate was too low. */
            mask_avg += GCONST(0.25f32);
            if mask_avg > 0.0 {
                /* Something went really wrong in the original calculations,
                   disabling masking. */
                mask_avg = 0.0;
                diff = 0.0;
                for v in surround_dynalloc[..mask_end as usize].iter_mut() {
                    *v = 0.0;
                }
            } else {
                for i in 0..mask_end {
                    surround_dynalloc[i as usize] =
                        MAXG(0.0f32, surround_dynalloc[i as usize] - GCONST(0.25f32));
                }
            }
        }
        mask_avg += GCONST(0.2f32);
        /* Convert to 1/64th units used for the trim */
        surround_trim = 64.0f32 * diff;
        surround_masking = mask_avg;
    }
    /* Temporal VBR (but not for LFE) */
    if st.lfe == 0 {
        let mut follow: celt_glog = -QCONST32(10.0f32, DB_SHIFT - 5);
        let mut frame_avg: opus_val32 = 0.0;
        let offset: celt_glog = if shortBlocks != 0 {
            HALF32(SHL32(LM as opus_val32, DB_SHIFT - 5))
        } else {
            0.0
        };
        for i in start..end {
            follow = MAXG(
                follow - QCONST32(1.0f32, DB_SHIFT - 5),
                SHR32(bandLogE[i as usize], 5) - offset,
            );
            if C == 2 {
                follow = MAXG(follow, SHR32(bandLogE[(i + nbEBands) as usize], 5) - offset);
            }
            frame_avg += follow;
        }
        frame_avg /= (end - start) as opus_val32;
        temporal_vbr = SUB32(SHL32(frame_avg, 5), st.spec_avg);
        temporal_vbr = MING(GCONST(3.0f32), MAXG(-GCONST(1.5f32), temporal_vbr));
        st.spec_avg += MULT16_32_Q15(QCONST16(0.02f32, 15), temporal_vbr);
    }

    if secondMdct == 0 {
        bandLogE2[..(C * nbEBands) as usize].copy_from_slice(&bandLogE[..(C * nbEBands) as usize]);
    }

    /* Last chance to catch any transient we might have missed in the
       time-domain analysis */
    if LM > 0
        && enc.ec_tell() + 3 <= total_bits
        && isTransient == 0
        && st.complexity >= 5
        && st.lfe == 0
        && hybrid == 0
    {
        if patch_transient_decision(&bandLogE, &st.oldBandE, nbEBands, start, end, C) != 0 {
            isTransient = 1;
            shortBlocks = M;
            compute_mdcts(mode, shortBlocks, &in_, &mut freq, C, CC, LM, st.upsample, st.arch);
            compute_band_energies(mode, &freq, &mut bandE, effEnd, C, LM, st.arch);
            amp2Log2(mode, effEnd, end, &bandE, &mut bandLogE, C);
            /* Compensate for the scaling of short vs long mdcts */
            for c in 0..C {
                for i in 0..end {
                    bandLogE2[(nbEBands * c + i) as usize] += HALF32(SHL32(LM as opus_val32, DB_SHIFT));
                }
            }
            tf_estimate = QCONST16(0.2f32, 14);
        }
    }

    if LM > 0 && enc.ec_tell() + 3 <= total_bits {
        ec_enc_bit_logp(enc, isTransient, 3);
    }

    let mut X = vec![0.0f32 as celt_norm; (C * N) as usize]; /* Interleaved normalised MDCTs */

    /* Band normalisation */
    normalise_bands(mode, &freq, &mut X, &bandE, effEnd, C, M);

    enable_tf_analysis = (effectiveBytes >= 15 * C
        && hybrid == 0
        && st.complexity >= 2
        && st.lfe == 0
        && toneishness < QCONST32(0.98f32, 29)) as i32;

    let mut offsets = vec![0i32; nbEBands as usize];
    let mut importance = vec![0i32; nbEBands as usize];
    let mut spread_weight = vec![0i32; nbEBands as usize];

    maxDepth = dynalloc_analysis(
        &bandLogE,
        &bandLogE2,
        &st.oldBandE,
        nbEBands,
        start,
        end,
        C,
        &mut offsets,
        st.lsb_depth,
        &mode.logN,
        isTransient,
        st.vbr,
        st.constrained_vbr,
        eBands,
        LM,
        effectiveBytes,
        &mut tot_boost,
        st.lfe,
        &surround_dynalloc,
        &st.analysis,
        &mut importance,
        &mut spread_weight,
        tone_freq,
        toneishness,
    );

    let mut tf_res = vec![0i32; nbEBands as usize];
    /* Disable variable tf resolution for hybrid and at very low bitrate */
    if enable_tf_analysis != 0 {
        let lambda: i32;
        lambda = IMAX(80, 20480 / effectiveBytes + 2);
        tf_select = tf_analysis(
            mode,
            effEnd,
            isTransient,
            &mut tf_res,
            lambda,
            &X,
            N,
            LM,
            tf_estimate,
            tf_chan,
            &importance,
        );
        for i in effEnd..end {
            tf_res[i as usize] = tf_res[(effEnd - 1) as usize];
        }
    } else if hybrid != 0 && weak_transient != 0 {
        /* For weak transients, we rely on the fact that improving time resolution using
           TF on a long window is imperfect. */
        for i in 0..end {
            tf_res[i as usize] = 1;
        }
        tf_select = 0;
    } else if hybrid != 0 && effectiveBytes < 15 && st.silk_info.signalType != 2 {
        /* For low bitrate hybrid, we force temporal resolution to 5 ms rather than 2.5 ms. */
        for i in 0..end {
            tf_res[i as usize] = 0;
        }
        tf_select = isTransient;
    } else {
        for i in 0..end {
            tf_res[i as usize] = isTransient;
        }
        tf_select = 0;
    }

    let mut error = vec![0.0f32 as celt_glog; (C * nbEBands) as usize];
    c = 0;
    loop {
        for i in start..end {
            /* When the energy is stable, slightly bias energy quantization towards
               the previous error to make the gain more stable. */
            if ABS32(SUB32(
                bandLogE[(i + c * nbEBands) as usize],
                st.oldBandE[(i + c * nbEBands) as usize],
            )) < GCONST(2.0f32)
            {
                bandLogE[(i + c * nbEBands) as usize] -= MULT16_32_Q15(
                    QCONST16(0.25f32, 15),
                    st.energyError[(i + c * nbEBands) as usize],
                );
            }
        }
        c += 1;
        if c >= C {
            break;
        }
    }
    quant_coarse_energy(
        mode,
        start,
        end,
        effEnd,
        &bandLogE,
        &mut st.oldBandE,
        total_bits as opus_uint32,
        &mut error,
        enc,
        C,
        LM,
        nbAvailableBytes,
        st.force_intra,
        &mut st.delayedIntra,
        (st.complexity >= 4) as i32,
        st.loss_rate,
        st.lfe,
    );

    tf_encode(start, end, isTransient, &mut tf_res, LM, tf_select, enc);

    if enc.ec_tell() + 4 <= total_bits {
        if st.lfe != 0 {
            st.tapset_decision = 0;
            st.spread_decision = super::bands::SPREAD_NORMAL;
        } else if hybrid != 0 {
            if st.complexity == 0 {
                st.spread_decision = super::bands::SPREAD_NONE;
            } else if isTransient != 0 {
                st.spread_decision = super::bands::SPREAD_NORMAL;
            } else {
                st.spread_decision = super::bands::SPREAD_AGGRESSIVE;
            }
        } else if shortBlocks != 0 || st.complexity < 3 || nbAvailableBytes < 10 * C {
            if st.complexity == 0 {
                st.spread_decision = super::bands::SPREAD_NONE;
            } else {
                st.spread_decision = super::bands::SPREAD_NORMAL;
            }
        } else {
            /* Disable new spreading+tapset estimator until we can show it works
               better than the old one. */
            st.spread_decision = spreading_decision(
                mode,
                &X,
                &mut st.tonal_average,
                st.spread_decision,
                &mut st.hf_average,
                &mut st.tapset_decision,
                (pf_on != 0 && shortBlocks == 0) as i32,
                effEnd,
                C,
                M,
                &spread_weight,
            );
        }
        ec_enc_icdf(enc, st.spread_decision, &spread_icdf, 5);
    } else {
        st.spread_decision = super::bands::SPREAD_NORMAL;
    }

    /* For LFE, everything interesting is in the first band */
    if st.lfe != 0 {
        offsets[0] = IMIN(8, effectiveBytes / 3);
    }
    let mut cap = vec![0i32; nbEBands as usize];
    init_caps(mode, &mut cap, LM, C);

    dynalloc_logp = 6;
    total_bits <<= BITRES;
    total_boost = 0;
    tell = ec_tell_frac(enc) as opus_int32;
    for i in start..end {
        let width;
        let quanta;
        let mut dynalloc_loop_logp: i32;
        let mut boost;
        let mut j: i32;
        width = C * ((eBands[(i + 1) as usize] as i32) - (eBands[i as usize] as i32)) << LM;
        /* quanta is 6 bits, but no more than 1 bit/sample
           and no less than 1/8 bit/sample */
        quanta = IMIN(width << BITRES, IMAX(6 << BITRES, width));
        dynalloc_loop_logp = dynalloc_logp;
        boost = 0;
        j = 0;
        loop {
            if !(tell + (dynalloc_loop_logp << BITRES) < total_bits - total_boost
                && boost < cap[i as usize])
            {
                break;
            }
            let flag: i32;
            flag = (j < offsets[i as usize]) as i32;
            ec_enc_bit_logp(enc, flag, dynalloc_loop_logp as u32);
            tell = ec_tell_frac(enc) as opus_int32;
            if flag == 0 {
                break;
            }
            boost += quanta;
            total_boost += quanta;
            dynalloc_loop_logp = 1;
            j += 1;
        }
        /* Making dynalloc more likely */
        if j != 0 {
            dynalloc_logp = IMAX(2, dynalloc_logp - 1);
        }
        offsets[i as usize] = boost;
    }

    if C == 2 {
        /* 0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19  20  off*/
        static intensity_thresholds: [opus_val16; 21] = [
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 16.0, 24.0, 36.0, 44.0, 50.0, 56.0, 62.0,
            67.0, 72.0, 79.0, 88.0, 106.0, 134.0,
        ];
        static intensity_histeresis: [opus_val16; 21] = [
            1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 4.0,
            5.0, 6.0, 8.0, 8.0,
        ];

        /* Always use MS for 2.5 ms frames until we can do a better analysis */
        if LM != 0 {
            dual_stereo = stereo_analysis(mode, &X, LM, N);
        }

        st.intensity = hysteresis_decision(
            (equiv_rate / 1000) as opus_val16,
            &intensity_thresholds,
            &intensity_histeresis,
            21,
            st.intensity,
        );
        st.intensity = IMIN(end, IMAX(start, st.intensity));
    }

    alloc_trim = 5;
    if tell + (6 << BITRES) <= total_bits - total_boost {
        if start > 0 || st.lfe != 0 {
            st.stereo_saving = 0.0;
            alloc_trim = 5;
        } else {
            alloc_trim = alloc_trim_analysis(
                mode,
                &X,
                &bandLogE,
                end,
                LM,
                C,
                N,
                &st.analysis,
                &mut st.stereo_saving,
                tf_estimate,
                st.intensity,
                surround_trim,
                equiv_rate,
                st.arch,
            );
        }
        ec_enc_icdf(enc, alloc_trim, &trim_icdf, 7);
        tell = ec_tell_frac(enc) as opus_int32;
    }

    /* In VBR mode the frame size must not be reduced so much that it would
        result in the encoder running out of bits. */
    min_allowed = ((tell + total_boost + (1 << (BITRES + 3)) - 1) >> (BITRES + 3)) + 2;
    /* Take into account the 37 bits we need to have left in the packet to
       signal a redundant frame in hybrid mode. */
    if hybrid != 0 {
        min_allowed = IMAX(
            min_allowed,
            (tell0_frac + (37 << BITRES) + total_boost + (1 << (BITRES + 3)) - 1)
                >> (BITRES + 3),
        );
    }
    /* Variable bitrate */
    if vbr_rate > 0 {
        let alpha: opus_val16;
        let mut delta: opus_int32;
        /* The target rate in 8th bits per frame */
        let mut target: opus_int32;
        let mut base_target: opus_int32;
        let lm_diff = mode.maxLM - LM;

        /* Don't attempt to use more than 510 kb/s, even for frames smaller than 20 ms. */
        nbCompressedBytes = IMIN(nbCompressedBytes, packet_size_cap >> (3 - LM));
        if hybrid == 0 {
            base_target = vbr_rate - ((40 * C + 20) << BITRES);
        } else {
            base_target = IMAX(0, vbr_rate - ((9 * C + 4) << BITRES));
        }

        if st.constrained_vbr != 0 {
            base_target += st.vbr_offset >> lm_diff;
        }

        if hybrid == 0 {
            target = compute_vbr(
                mode,
                &st.analysis,
                base_target,
                LM,
                equiv_rate,
                st.lastCodedBands,
                C,
                st.intensity,
                st.constrained_vbr,
                st.stereo_saving,
                tot_boost,
                tf_estimate,
                pitch_change,
                maxDepth,
                st.lfe,
                st.energy_mask.is_some() as i32,
                surround_masking,
                temporal_vbr,
            );
        } else {
            target = base_target;
            /* Tonal frames (offset<100) need more bits than noisy (offset>100) ones. */
            if st.silk_info.offset < 100 {
                target += 12 << BITRES >> (3 - LM);
            }
            if st.silk_info.offset > 100 {
                target -= 18 << BITRES >> (3 - LM);
            }
            /* Boosting bitrate on transients and vowels with significant temporal
               spikes. */
            target += MULT16_16_Q14(
                tf_estimate - QCONST16(0.25f32, 14),
                (50 << BITRES) as opus_val32,
            ) as opus_int32;
            /* If we have a strong transient, let's make sure it has enough bits to code
               the first two bands, so that it can use folding rather than noise. */
            if tf_estimate > QCONST16(0.7f32, 14) {
                target = IMAX(target, 50 << BITRES);
            }
        }
        /* The current offset is removed from the target and the space used
            so far is added*/
        target = target + tell;

        nbAvailableBytes = (target + (1 << (BITRES + 2))) >> (BITRES + 3);
        nbAvailableBytes = IMAX(min_allowed, nbAvailableBytes);
        nbAvailableBytes = IMIN(nbCompressedBytes, nbAvailableBytes);

        /* By how much did we "miss" the target on that frame */
        delta = target - vbr_rate;

        target = nbAvailableBytes << (BITRES + 3);

        /*If the frame is silent we don't adjust our drift, otherwise
          the encoder will shoot to very high rates after hitting a
          span of silence, but we do allow the bitres to refill. */
        if silence != 0 {
            nbAvailableBytes = 2;
            target = 2 * 8 << BITRES;
            delta = 0;
        }

        if st.vbr_count < 970 {
            st.vbr_count += 1;
            alpha = celt_rcp(SHL32(EXTEND32((st.vbr_count + 20) as opus_val32), 16));
        } else {
            alpha = QCONST16(0.001f32, 15);
        }
        /* How many bits have we used in excess of what we're allowed */
        if st.constrained_vbr != 0 {
            st.vbr_reservoir += target - vbr_rate;
        }

        /* Compute the offset we need to apply in order to reach the target */
        if st.constrained_vbr != 0 {
            st.vbr_drift += MULT16_32_Q15(
                alpha,
                (delta * (1 << lm_diff) - st.vbr_offset - st.vbr_drift) as opus_val32,
            ) as opus_int32;
            st.vbr_offset = -st.vbr_drift;
        }

        if st.constrained_vbr != 0 && st.vbr_reservoir < 0 {
            /* We're under the min value -- increase rate */
            let adjust = (-st.vbr_reservoir) / (8 << BITRES);
            /* Unless we're just coding silence */
            nbAvailableBytes += if silence != 0 { 0 } else { adjust };
            st.vbr_reservoir = 0;
        }
        nbCompressedBytes = IMIN(nbCompressedBytes, nbAvailableBytes);
        /* This moves the raw bits to take into account the new compressed size */
        ec_enc_shrink(enc, nbCompressedBytes as opus_uint32);
    }

    /* Bit allocation */
    let mut fine_quant = vec![0i32; nbEBands as usize];
    let mut pulses = vec![0i32; nbEBands as usize];
    let mut fine_priority = vec![0i32; nbEBands as usize];

    /* bits =           packet size                    - where we are - safety*/
    bits = (((nbCompressedBytes * 8) as opus_int32) << BITRES) - (ec_tell_frac(enc) as opus_int32) - 1;
    anti_collapse_rsv = if isTransient != 0 && LM >= 2 && bits >= ((LM + 2) << BITRES) {
        1 << BITRES
    } else {
        0
    };
    bits -= anti_collapse_rsv;
    signalBandwidth = end - 1;
    /* `#ifndef DISABLE_FLOAT_API` */
    if st.analysis.valid != 0 {
        let min_bandwidth;
        if equiv_rate < 32000 * C {
            min_bandwidth = 13;
        } else if equiv_rate < 48000 * C {
            min_bandwidth = 16;
        } else if equiv_rate < 60000 * C {
            min_bandwidth = 18;
        } else if equiv_rate < 80000 * C {
            min_bandwidth = 19;
        } else {
            min_bandwidth = 20;
        }
        signalBandwidth = IMAX(st.analysis.bandwidth, min_bandwidth);
    }
    if st.lfe != 0 {
        signalBandwidth = 1;
    }
    codedBands = clt_compute_allocation(
        mode,
        start,
        end,
        &offsets,
        &cap,
        alloc_trim,
        &mut st.intensity,
        &mut dual_stereo,
        bits,
        &mut balance,
        &mut pulses,
        &mut fine_quant,
        &mut fine_priority,
        C,
        LM,
        enc,
        1,
        st.lastCodedBands,
        signalBandwidth,
    );
    if st.lastCodedBands != 0 {
        st.lastCodedBands = IMIN(st.lastCodedBands + 1, IMAX(st.lastCodedBands - 1, codedBands));
    } else {
        st.lastCodedBands = codedBands;
    }

    quant_fine_energy(mode, start, end, &mut st.oldBandE, &mut error, None, &fine_quant, enc, C);
    for v in st.energyError[..(nbEBands * CC) as usize].iter_mut() {
        *v = 0.0;
    }

    /* Residual quantisation */
    let mut collapse_masks = vec![0u8; (C * nbEBands) as usize];
    {
        let (x0, x1) = X.split_at_mut(N as usize);
        let y = if C == 2 { Some(x1) } else { None };
        quant_all_bands(
            1,
            mode,
            start,
            end,
            x0,
            y,
            &mut collapse_masks,
            &bandE,
            &pulses,
            shortBlocks,
            st.spread_decision,
            dual_stereo,
            st.intensity,
            &tf_res,
            nbCompressedBytes * (8 << BITRES) - anti_collapse_rsv,
            balance,
            enc,
            LM,
            codedBands,
            &mut st.rng,
            st.complexity,
            st.arch,
            st.disable_inv,
        );
    }

    if anti_collapse_rsv > 0 {
        let anti_collapse_on = (st.consec_transient < 2) as i32;
        ec_enc_bits(enc, anti_collapse_on as opus_uint32, 1);
    }
    /* `#ifdef ENABLE_QEXT` is not compiled: `qext_bytes == 0` always. */
    quant_energy_finalise(
        mode,
        start,
        end,
        Some(&mut st.oldBandE[..]),
        &mut error,
        &fine_quant,
        &fine_priority,
        nbCompressedBytes * 8 - enc.ec_tell(),
        enc,
        C,
    );
    c = 0;
    loop {
        for i in start..end {
            st.energyError[(i + c * nbEBands) as usize] = MAXG(
                -GCONST(0.5f32),
                MING(GCONST(0.5f32), error[(i + c * nbEBands) as usize]),
            );
        }
        c += 1;
        if c >= C {
            break;
        }
    }
    if silence != 0 {
        for i in 0..(C * nbEBands) as usize {
            st.oldBandE[i] = -GCONST(28.0f32);
        }
    }

    /* `#ifdef RESYNTH` is not compiled. */

    st.prefilter_period = pitch_index;
    st.prefilter_gain = gain1;
    st.prefilter_tapset = prefilter_tapset;

    if CC == 2 && C == 1 {
        recopy_bande(&mut st.oldBandE, nbEBands);
    }

    if isTransient == 0 {
        let n = (CC * nbEBands) as usize;
        st.oldLogE2[..n].copy_from_slice(&st.oldLogE[..n]);
        st.oldLogE[..n].copy_from_slice(&st.oldBandE[..n]);
    } else {
        for i in 0..(CC * nbEBands) as usize {
            st.oldLogE[i] = MING(st.oldLogE[i], st.oldBandE[i]);
        }
    }
    /* In case start or end were to change */
    c = 0;
    loop {
        for i in 0..start {
            st.oldBandE[(c * nbEBands + i) as usize] = 0.0;
            st.oldLogE[(c * nbEBands + i) as usize] = -GCONST(28.0f32);
            st.oldLogE2[(c * nbEBands + i) as usize] = -GCONST(28.0f32);
        }
        for i in end..nbEBands {
            st.oldBandE[(c * nbEBands + i) as usize] = 0.0;
            st.oldLogE[(c * nbEBands + i) as usize] = -GCONST(28.0f32);
            st.oldLogE2[(c * nbEBands + i) as usize] = -GCONST(28.0f32);
        }
        c += 1;
        if c >= CC {
            break;
        }
    }

    if isTransient != 0 || transient_got_disabled != 0 {
        st.consec_transient += 1;
    } else {
        st.consec_transient = 0;
    }
    st.rng = enc.rng;

    /* If there's any room left (can only happen for very high rates),
       it's already filled with zeros */
    ec_enc_done(enc);
    /* `#if defined(CUSTOM_MODES) || defined(ENABLE_OPUS_CUSTOM_API)` */
    if st.signalling != 0 {
        nbCompressedBytes += 1;
    }

    let error_code = enc.ec_get_error();
    if use_local {
        let n = _enc_storage.storage as usize;
        compressed[signal_offset..signal_offset + n].copy_from_slice(&_enc_storage.buf[..n]);
    }
    if error_code != 0 {
        OPUS_INTERNAL_ERROR
    } else {
        nbCompressedBytes
    }
}

/// `OPUS_COPY(&oldBandE[nbEBands], oldBandE, nbEBands)` in the mono-downmix
/// case.  Split out because `copy_within` needs a `&mut` borrow of the vec.
#[inline(always)]
fn recopy_bande(oldBandE: &mut [celt_glog], nbEBands: i32) {
    oldBandE.copy_within(0..nbEBands as usize, nbEBands as usize);
}

/* ------------------------------------------------------------------------- *
 * opus_custom_encode / opus_custom_encode24 / opus_custom_encode_float
 * ------------------------------------------------------------------------- */

/// `opus_custom_encode()` (`celt/celt_encoder.c:2843`, float build).
pub fn opus_custom_encode(
    st: &mut OpusCustomEncoder,
    pcm: &[opus_int16],
    frame_size: i32,
    compressed: &mut [u8],
    nbCompressedBytes: i32,
) -> i32 {
    let ret: i32;
    let C: i32;
    let N: i32;

    if pcm.is_empty() {
        return OPUS_BAD_ARG;
    }

    C = st.channels;
    N = frame_size;
    let mut in_ = vec![0.0f32 as opus_res; (C * N) as usize];

    for j in 0..(C * N) as usize {
        in_[j] = INT16TORES(pcm[j]);
    }

    ret = celt_encode_with_ec(st, &in_, frame_size, compressed, nbCompressedBytes, None);
    /* `#ifdef RESYNTH` is not compiled. */
    ret
}

/// `opus_custom_encode24()` (`celt/celt_encoder.c:2876`, float build).
pub fn opus_custom_encode24(
    st: &mut OpusCustomEncoder,
    pcm: &[opus_int32],
    frame_size: i32,
    compressed: &mut [u8],
    nbCompressedBytes: i32,
) -> i32 {
    let ret: i32;
    let C: i32;
    let N: i32;

    if pcm.is_empty() {
        return OPUS_BAD_ARG;
    }

    C = st.channels;
    N = frame_size;
    let mut in_ = vec![0.0f32 as opus_res; (C * N) as usize];

    for j in 0..(C * N) as usize {
        in_[j] = INT24TORES(pcm[j]);
    }

    ret = celt_encode_with_ec(st, &in_, frame_size, compressed, nbCompressedBytes, None);
    ret
}

/// `opus_custom_encode_float()` (`celt/celt_encoder.c:2906`, float build).
pub fn opus_custom_encode_float(
    st: &mut OpusCustomEncoder,
    pcm: &[f32],
    frame_size: i32,
    compressed: &mut [u8],
    nbCompressedBytes: i32,
) -> i32 {
    /* `# if !defined(FIXED_POINT)`: the float PCM is passed through directly. */
    celt_encode_with_ec(st, pcm, frame_size, compressed, nbCompressedBytes, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::modes::mode48000_960_120;

    fn new_encoder(CC: i32) -> Box<OpusCustomEncoder> {
        let mut err = 0i32;
        opus_custom_encoder_create(Box::new(mode48000_960_120()), CC, &mut err)
            .expect("mode48000_960_120 must initialise")
    }

    #[test]
    fn encoder_size_matches_reference() {
        /* Verified against reference/lib-ref/libopus.a: 5164 / 10076. */
        assert_eq!(SIZEOF_CELT_ENCODER, 256);
        assert_eq!(celt_encoder_get_size(1), 5164);
        assert_eq!(celt_encoder_get_size(2), 10076);
        let m = mode48000_960_120();
        assert_eq!(opus_custom_encoder_get_size(&m, 1), 5164);
        assert_eq!(opus_custom_encoder_get_size(&m, 2), 10076);
    }

    #[test]
    fn init_sets_the_c_defaults() {
        let st = new_encoder(2);
        assert_eq!(st.channels, 2);
        assert_eq!(st.stream_channels, 2);
        assert_eq!(st.upsample, 1);
        assert_eq!(st.start, 0);
        assert_eq!(st.end, st.mode.effEBands);
        assert_eq!(st.signalling, 1);
        assert_eq!(st.arch, OPUS_ARCH);
        assert_eq!(st.constrained_vbr, 1);
        assert_eq!(st.clip, 1);
        assert_eq!(st.bitrate, OPUS_BITRATE_MAX);
        assert_eq!(st.vbr, 0);
        assert_eq!(st.force_intra, 0);
        assert_eq!(st.complexity, 5);
        assert_eq!(st.lsb_depth, 24);
        /* OPUS_RESET_STATE */
        assert_eq!(st.rng, 0);
        assert_eq!(st.delayedIntra, 1.0);
        assert_eq!(st.spread_decision, super::super::bands::SPREAD_NORMAL);
        assert_eq!(st.tonal_average, 256);
        assert_eq!(st.hf_average, 0);
        assert_eq!(st.tapset_decision, 0);
        assert_eq!(st.vbr_offset, 0);
        assert!(st.oldBandE.iter().all(|&v| v == 0.0));
        assert!(st.oldLogE.iter().all(|&v| v == -GCONST(28.0f32)));
        assert!(st.oldLogE2.iter().all(|&v| v == -GCONST(28.0f32)));
        assert!(st.energy_mask.is_none());
    }

    #[test]
    fn ctl_validates_and_reads_back() {
        let mut st = new_encoder(1);
        assert_eq!(opus_custom_encoder_ctl(&mut st, OPUS_SET_COMPLEXITY_REQUEST, &mut 11), OPUS_BAD_ARG);
        assert_eq!(opus_custom_encoder_ctl(&mut st, OPUS_SET_COMPLEXITY_REQUEST, &mut 0), OPUS_OK);
        assert_eq!(st.complexity, 0);
        let mut nbe = st.mode.nbEBands;
        assert_eq!(
            opus_custom_encoder_ctl(&mut st, super::super::celt::CELT_SET_START_BAND_REQUEST, &mut nbe),
            OPUS_BAD_ARG
        );
        assert_eq!(
            opus_custom_encoder_ctl(&mut st, super::super::celt::CELT_SET_END_BAND_REQUEST, &mut 0),
            OPUS_BAD_ARG
        );
        let mut v = 0i32;
        assert_eq!(opus_custom_encoder_ctl(&mut st, OPUS_GET_LSB_DEPTH_REQUEST, &mut v), OPUS_OK);
        assert_eq!(v, 24);
        assert_eq!(
            opus_custom_encoder_ctl(&mut st, OPUS_SET_BITRATE_REQUEST, &mut 100),
            OPUS_BAD_ARG
        );
        assert_eq!(opus_custom_encoder_ctl(&mut st, 999999, &mut 0), OPUS_UNIMPLEMENTED);
    }

    #[test]
    fn reset_state_clears_from_rng_onwards() {
        let mut st = new_encoder(2);
        st.rng = 0xdeadbeef;
        st.delayedIntra = 3.5;
        st.oldBandE[0] = 1.0;
        st.oldLogE[0] = 1.0;
        st.oldLogE2[0] = 1.0;
        st.energyError[0] = 1.0;
        st.in_mem[0] = 1.0;
        st.prefilter_mem[0] = 1.0;
        st.prefilter_period = 42;
        st.consec_transient = 7;
        opus_custom_encoder_ctl(&mut st, OPUS_RESET_STATE, &mut 0);
        assert_eq!(st.rng, 0);
        assert_eq!(st.delayedIntra, 1.0);
        assert_eq!(st.prefilter_period, 0);
        assert_eq!(st.consec_transient, 0);
        assert_eq!(st.oldBandE[0], 0.0);
        assert_eq!(st.oldLogE[0], -GCONST(28.0f32));
        assert_eq!(st.oldLogE2[0], -GCONST(28.0f32));
        assert_eq!(st.energyError[0], 0.0);
        assert_eq!(st.in_mem[0], 0.0);
        assert_eq!(st.prefilter_mem[0], 0.0);
    }

    #[test]
    fn median_of_3_and_5() {
        assert_eq!(median_of_3(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(median_of_3(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median_of_3(&[2.0, 3.0, 1.0]), 2.0);
        assert_eq!(median_of_3(&[-1.0, -5.0, -3.0]), -3.0);
        /* Every permutation of 5 distinct values has the same median. */
        let v = [3.0f32, 1.0, 4.0, 1.5, 2.0];
        let mut perms = 0;
        for a in 0..5 {
            for b in 0..5 {
                for c in 0..5 {
                    for d in 0..5 {
                        for e in 0..5 {
                            let idx = [a, b, c, d, e];
                            let mut seen = [false; 5];
                            let mut ok = true;
                            for &k in idx.iter() {
                                if seen[k] {
                                    ok = false;
                                }
                                seen[k] = true;
                            }
                            if !ok {
                                continue;
                            }
                            let x: Vec<f32> = idx.iter().map(|&k| v[k]).collect();
                            assert_eq!(median_of_5(&x), 2.0);
                            perms += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(perms, 120);
    }

    #[test]
    fn l1_metric_matches_definition() {
        let tmp = [1.0f32, -2.0, 3.0];
        /* LM == 0 -> L1 = 1 + 2 + 3 = 6 */
        assert_eq!(l1_metric(&tmp, 3, 0, 0.0), 6.0);
        /* L1 = L1 + (LM*bias)*L1 */
        let got = l1_metric(&tmp, 3, 2, 0.25);
        assert_eq!(got, 6.0 + (2.0f32 * 0.25) * 6.0);
    }

    #[test]
    fn preemphasis_fast_path() {
        let pcmp = [0.5f32, -0.25];
        let coef = [0.85f32, 0.0, 1.0, 0.0];
        let mut inp = [0.0f32; 2];
        let mut mem = 0.1f32;
        celt_preemphasis(&pcmp, &mut inp, 2, 1, 1, &coef, &mut mem, 0);
        let mut m = 0.1f32;
        let mut exp = [0.0f32; 2];
        for i in 0..2 {
            let x = RES2SIG(pcmp[i]);
            exp[i] = x - m;
            m = MULT16_32_Q15(coef[0], x);
        }
        assert_eq!(inp, exp);
        assert_eq!(mem, m);
    }

    #[test]
    fn preemphasis_second_order_path() {
        let pcmp = [0.1f32, 0.2, 0.3];
        let coef = [0.85f32, -0.1799926758, 1.0, 0.0];
        let mut inp = [0.0f32; 3];
        let mut mem = 0.0f32;
        celt_preemphasis(&pcmp, &mut inp, 3, 1, 1, &coef, &mut mem, 0);
        let mut m = 0.0f32;
        let mut exp = [0.0f32; 3];
        for i in 0..3 {
            let x = RES2SIG(pcmp[i]);
            let tmp = SHL32(MULT16_32_Q15(coef[2], x), 15 - SIG_SHIFT);
            exp[i] = tmp + m;
            m = MULT16_32_Q15(coef[1], exp[i]) - MULT16_32_Q15(coef[0], tmp);
        }
        assert_eq!(inp, exp);
        assert_eq!(mem, m);
    }

    #[test]
    fn preemphasis_clips_and_upsamples() {
        let pcmp = [3.0f32, -3.0];
        let coef = [0.85f32, 0.0, 1.0, 0.0];
        let mut inp = [0.0f32; 4];
        let mut mem = 0.0f32;
        celt_preemphasis(&pcmp, &mut inp, 4, 1, 2, &coef, &mut mem, 1);
        let mut m = 0.0f32;
        let mut exp = [0.0f32; 4];
        exp[0] = 65536.0;
        exp[2] = -65536.0;
        for i in 0..4 {
            let x = exp[i];
            exp[i] = x - m;
            m = MULT16_32_Q15(coef[0], x);
        }
        assert_eq!(inp, exp);
        assert_eq!(mem, m);
    }

    #[test]
    fn patch_transient_decision_thresholds() {
        let nbEBands = 21;
        let newE = vec![0.0f32; nbEBands];
        let oldE = vec![0.0f32; nbEBands];
        assert_eq!(patch_transient_decision(&newE, &oldE, nbEBands as i32, 0, nbEBands as i32, 1), 0);
        let newE2 = vec![5.0f32; nbEBands];
        assert_eq!(patch_transient_decision(&newE2, &oldE, nbEBands as i32, 0, nbEBands as i32, 1), 1);
    }

    #[test]
    fn tone_lpc_finds_a_sinusoid() {
        let len = 256i32;
        let mut x = vec![0.0f32; len as usize];
        for i in 0..len as usize {
            x[i] = ((2.0f64 * std::f64::consts::PI * i as f64 / 40.0).sin() * 30000.0) as f32;
        }
        let mut lpc = [0.0f32; 2];
        assert_eq!(tone_lpc(&x, len, 1, &mut lpc), 0);
        /* complex roots */
        assert!(lpc[0] * lpc[0] + 3.999999f32 * lpc[1] < 0.0);
        /* the pole radius is close to one for a clean tone */
        assert!(-lpc[1] > 0.9);
    }

    #[test]
    fn tone_detect_reports_a_frequency() {
        let n = 480i32;
        let mut sig = vec![0.0f32; n as usize];
        for i in 0..n as usize {
            sig[i] = ((2.0f64 * std::f64::consts::PI * i as f64 / 100.0).sin() * 10000.0) as f32;
        }
        let mut toneishness = 0.0f32;
        let freq = tone_detect(&sig, 1, n, &mut toneishness, 48000);
        assert!(freq.is_finite());
        /* ~2*pi/100 for the tone branch of run_prefilter */
        assert!((freq - 0.0628).abs() < 0.02);
        assert!(toneishness > 0.99);
    }

    #[test]
    fn tf_encode_applies_the_select_table() {
        let mut enc = ec_enc_init(64);
        let mut tf_res = vec![0i32; 21];
        tf_encode(0, 21, 0, &mut tf_res, 3, 0, &mut enc);
        /* non-transient, tf_select ends up 0 -> tf_select_table[3][0] */
        assert!(tf_res.iter().all(|&v| v == tf_select_table[3][0] as i32));
        assert_eq!(tf_res[0], 0);

        let mut enc = ec_enc_init(64);
        let mut tf_res = vec![0i32; 21];
        tf_encode(0, 21, 1, &mut tf_res, 3, 0, &mut enc);
        /* transient -> tf_select_table[3][4] == 3 */
        assert!(tf_res.iter().all(|&v| v == tf_select_table[3][4] as i32));
        assert_eq!(tf_res[0], 3);
    }

    #[test]
    fn compute_vbr_is_bounded_by_twice_the_base() {
        let m = mode48000_960_120();
        let analysis = AnalysisInfo::default();
        for base in [1000i32, 8000, 30000] {
            let t = compute_vbr(
                &m, &analysis, base, 3, 64000, 21, 1, 21, 0, 0.0, 0, 0.0, 0, -30.0, 0, 0, 0.0, 0.0,
            );
            assert!(t <= 2 * base);
        }
    }

    #[test]
    fn stereo_analysis_picks_dual_stereo_when_ms_is_expensive() {
        let m = mode48000_960_120();
        let n = 120usize;
        /* L == R -> MS is cheaper, so dual_stereo == 0. */
        let mut x = vec![0.0f32; 2 * n];
        for i in 0..n {
            x[i] = 0.5;
            x[n + i] = 0.5;
        }
        assert_eq!(stereo_analysis(&m, &x, 0, n as i32), 0);
        /* Uncorrelated channels -> the L1 model says L/R is cheaper. */
        let mut x = vec![0.0f32; 2 * n];
        for i in 0..n {
            x[i] = 1.0;
            x[n + i] = 0.0;
        }
        assert_eq!(stereo_analysis(&m, &x, 0, n as i32), 1);
    }

    #[test]
    fn custom_encode_returns_bad_arg_for_empty_pcm() {
        let mut st = new_encoder(1);
        let mut out = [0u8; 64];
        assert_eq!(opus_custom_encode(&mut st, &[], 120, &mut out, 64), OPUS_BAD_ARG);
        assert_eq!(opus_custom_encode_float(&mut st, &[], 120, &mut out, 64), OPUS_BAD_ARG);
        assert_eq!(celt_encode_with_ec(&mut st, &[], 120, &mut out, 64, None), OPUS_BAD_ARG);
    }

    #[test]
    fn celt_encoder_init_sets_the_upsample_factor() {
        for (rate, factor) in [(48000, 1), (24000, 2), (16000, 3), (12000, 4), (8000, 6)] {
            let mut st = Box::new(OpusCustomEncoder::zeroed());
            assert_eq!(celt_encoder_init(&mut st, rate, 1, OPUS_ARCH), OPUS_OK);
            assert_eq!(st.upsample, factor);
        }
        let mut st = Box::new(OpusCustomEncoder::zeroed());
        assert_eq!(opus_custom_encoder_init_arch(&mut st, Box::new(mode48000_960_120()), 3, 4), OPUS_BAD_ARG);
    }
}



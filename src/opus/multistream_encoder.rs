/* Copyright (c) 2011 Xiph.Org Foundation
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
//! Transliteration of `src/opus_multistream_encoder.c` from libopus 1.6.1 into
//! safe Rust.
//!
//! The per-stream `OpusEncoder` states and the surround scratch that C lays out
//! inside the `OpusMSEncoder` allocation are owned by [`OpusMSEncoder`] (see
//! [`super::multistream`]); everything else follows the C one-to-one, including
//! `surround_analysis()` (mapping family 1, > 2 channels), the surround /
//! ambisonics rate allocation and the `ctl` switch.
//!
//! The C `opus_multistream_encoder_init_impl()` takes a nullable `st` (NULL
//! returns the allocation size); the port keeps that shape with
//! `Option<&mut OpusMSEncoder>`.  The C
//! `opus_multistream_encoder_ctl_va_list()` variadic body becomes the scalar
//! `opus_multistream_encoder_ctl(&mut st, request, &mut value)`, and the one
//! request whose C argument is a pointer
//! (`OPUS_MULTISTREAM_GET_ENCODER_STATE`) is exposed as the typed
//! `OpusMSEncoder::get_encoder_state()`.

use crate::celt::arch::*;
use crate::celt::bands::compute_band_energies;
use crate::celt::celt::{bitrate_to_bits, bits_to_bitrate, resampling_factor, OPUS_SET_LFE_REQUEST};
use crate::celt::celt_encoder::celt_preemphasis;
use crate::celt::kernels::celt_inner_prod;
use crate::celt::mathops::{celt_log2, isqrt32};
use crate::celt::mdct::clt_mdct_forward;
use crate::celt::modes::CELTMode;
use crate::celt::quant_bands::amp2Log2;
use crate::ffi::*;
use crate::opus::encoder::analysis::AnalysisPcm;
use crate::opus::encoder::{
    frame_size_select, opus_encode_native, OpusEncoder, OPUS_GET_VOICE_RATIO_REQUEST,
    OPUS_SET_FORCE_MODE_REQUEST,
};
use crate::opus::multistream::{
    align, get_left_channel, get_mono_channel, get_right_channel, validate_layout, MappingType,
    MAX_OVERLAP, MS_ENCODER_ALIGN_SIZE,
};
use crate::opus::packet::MODE_CELT_ONLY;
use crate::opus::repacketizer::OpusRepacketizer;
use crate::silk::init_decoder::opus_select_arch;
use crate::types::opus_int32;

/// `struct OpusMSEncoder` lives in [`super::multistream`]; re-exported here so
/// that the encoder entry points and their state type share one module path.
pub use crate::opus::multistream::OpusMSEncoder;

/// `MAXG(a,b)` (`celt/arch.h:110`).
#[inline(always)]
fn MAXG(a: f32, b: f32) -> f32 {
    MAX32(a, b)
}

/* ------------------------------------------------------------------------- *
 * vorbis_mappings
 * ------------------------------------------------------------------------- */

/// `VorbisLayout` (`src/opus_multistream_encoder.c:46`).
struct VorbisLayout {
    nb_streams: i32,
    nb_coupled_streams: i32,
    mapping: [u8; 8],
}

/// `vorbis_mappings[8]` (`src/opus_multistream_encoder.c:53`); the index is
/// `nb_channels-1`.
static vorbis_mappings: [VorbisLayout; 8] = [
    VorbisLayout { nb_streams: 1, nb_coupled_streams: 0, mapping: [0, 0, 0, 0, 0, 0, 0, 0] }, /* 1: mono */
    VorbisLayout { nb_streams: 1, nb_coupled_streams: 1, mapping: [0, 1, 0, 0, 0, 0, 0, 0] }, /* 2: stereo */
    VorbisLayout { nb_streams: 2, nb_coupled_streams: 1, mapping: [0, 2, 1, 0, 0, 0, 0, 0] }, /* 3: 1-d surround */
    VorbisLayout { nb_streams: 2, nb_coupled_streams: 2, mapping: [0, 1, 2, 3, 0, 0, 0, 0] }, /* 4: quadraphonic */
    VorbisLayout { nb_streams: 3, nb_coupled_streams: 2, mapping: [0, 4, 1, 2, 3, 0, 0, 0] }, /* 5: 5-channel */
    VorbisLayout { nb_streams: 4, nb_coupled_streams: 2, mapping: [0, 4, 1, 2, 3, 5, 0, 0] }, /* 6: 5.1 */
    VorbisLayout { nb_streams: 4, nb_coupled_streams: 3, mapping: [0, 4, 1, 2, 3, 5, 6, 0] }, /* 7: 6.1 */
    VorbisLayout { nb_streams: 5, nb_coupled_streams: 3, mapping: [0, 6, 1, 2, 3, 4, 5, 7] }, /* 8: 7.1 */
];

/* ------------------------------------------------------------------------- *
 * validate_ambisonics / validate_encoder_layout
 * ------------------------------------------------------------------------- */

/// `validate_ambisonics()` (`src/opus_multistream_encoder.c:110`).
fn validate_ambisonics(
    nb_channels: i32,
    nb_streams: Option<&mut i32>,
    nb_coupled_streams: Option<&mut i32>,
) -> i32 {
    let order_plus_one;
    let acn_channels;
    let nondiegetic_channels;

    if nb_channels < 1 || nb_channels > 227 {
        return 0;
    }

    order_plus_one = isqrt32(nb_channels as u32) as i32;
    acn_channels = order_plus_one * order_plus_one;
    nondiegetic_channels = nb_channels - acn_channels;

    if nondiegetic_channels != 0 && nondiegetic_channels != 2 {
        return 0;
    }

    if let Some(ns) = nb_streams {
        *ns = acn_channels + (nondiegetic_channels != 0) as i32;
    }
    if let Some(nc) = nb_coupled_streams {
        *nc = (nondiegetic_channels != 0) as i32;
    }
    1
}

/// `validate_encoder_layout()` (`src/opus_multistream_encoder.c:133`).
fn validate_encoder_layout(layout: &crate::opus::multistream::ChannelLayout) -> i32 {
    let mut s = 0;
    while s < layout.nb_streams {
        if s < layout.nb_coupled_streams {
            if get_left_channel(layout, s, -1) == -1 {
                return 0;
            }
            if get_right_channel(layout, s, -1) == -1 {
                return 0;
            }
        } else {
            if get_mono_channel(layout, s, -1) == -1 {
                return 0;
            }
        }
        s += 1;
    }
    1
}

/* ------------------------------------------------------------------------- *
 * channel_pos / logSum
 * ------------------------------------------------------------------------- */

/// `channel_pos()` (`src/opus_multistream_encoder.c:152`).
fn channel_pos(channels: i32, pos: &mut [i32; 8]) {
    /* Position in the mix: 0 don't mix, 1: left, 2: center, 3:right */
    if channels == 4 {
        pos[0] = 1;
        pos[1] = 3;
        pos[2] = 1;
        pos[3] = 3;
    } else if channels == 3 || channels == 5 || channels == 6 {
        pos[0] = 1;
        pos[1] = 2;
        pos[2] = 3;
        pos[3] = 1;
        pos[4] = 3;
        pos[5] = 0;
    } else if channels == 7 {
        pos[0] = 1;
        pos[1] = 2;
        pos[2] = 3;
        pos[3] = 1;
        pos[4] = 3;
        pos[5] = 2;
        pos[6] = 0;
    } else if channels == 8 {
        pos[0] = 1;
        pos[1] = 2;
        pos[2] = 3;
        pos[3] = 1;
        pos[4] = 3;
        pos[5] = 1;
        pos[6] = 3;
        pos[7] = 0;
    }
}

/// `diff_table[17]` (`src/opus_multistream_encoder.c:198`).  C only initialises
/// the leading 9 entries, so the remaining 8 are zero.
static diff_table: [f32; 17] = [
    0.5000000, 0.2924813, 0.1609640, 0.0849625, 0.0437314, 0.0221971, 0.0111839, 0.0056136,
    0.0028123, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

/// `logSum()` (`src/opus_multistream_encoder.c:193`): a rough approximation of
/// `log2(2^a + 2^b)`.
fn logSum(a: celt_glog, b: celt_glog) -> celt_glog {
    let max;
    let diff;
    let frac;
    let low;

    if a > b {
        max = a;
        diff = SUB32(a, b);
    } else {
        max = b;
        diff = SUB32(b, a);
    }
    if !(diff < 8.0f32) {
        /* inverted to catch NaNs */
        return max;
    }
    low = (2.0f32 * diff).floor() as i32;
    frac = 2.0f32 * diff - low as f32;
    max + diff_table[low as usize]
        + MULT16_32_Q15(frac, SUB32(diff_table[(low + 1) as usize], diff_table[low as usize]))
}

/* ------------------------------------------------------------------------- *
 * surround_analysis
 * ------------------------------------------------------------------------- */

/// `opus_multistream_encode*()`'s PCM argument plus its
/// `opus_copy_channel_in_func` (`opus_copy_channel_in_short/int24/float`).
#[derive(Clone, Copy)]
pub enum MsEncoderPcm<'a> {
    /// `opus_copy_channel_in_short`
    Short(&'a [i16]),
    /// `opus_copy_channel_in_int24`
    Int24(&'a [i32]),
    /// `opus_copy_channel_in_float`
    Float(&'a [f32]),
}

impl<'a> MsEncoderPcm<'a> {
    /// The `(analysis_pcm, downmix_func)` pair that `opus_encode_native()`
    /// consumes.
    fn analysis(&self) -> AnalysisPcm<'a> {
        match *self {
            MsEncoderPcm::Short(x) => AnalysisPcm::Int(x),
            MsEncoderPcm::Int24(x) => AnalysisPcm::Int24(x),
            MsEncoderPcm::Float(x) => AnalysisPcm::Float(x),
        }
    }

    /// `opus_copy_channel_in_short/int24/float()`.
    fn copy_channel_in(
        &self,
        dst: &mut [opus_res],
        dst_stride: i32,
        src_stride: i32,
        src_channel: i32,
        frame_size: i32,
    ) {
        match *self {
            MsEncoderPcm::Short(src) => {
                for i in 0..frame_size {
                    dst[(i * dst_stride) as usize] =
                        INT16TORES(src[(i * src_stride + src_channel) as usize]);
                }
            }
            MsEncoderPcm::Int24(src) => {
                for i in 0..frame_size {
                    dst[(i * dst_stride) as usize] =
                        INT24TORES(src[(i * src_stride + src_channel) as usize]);
                }
            }
            MsEncoderPcm::Float(src) => {
                for i in 0..frame_size {
                    dst[(i * dst_stride) as usize] =
                        FLOAT2RES(src[(i * src_stride + src_channel) as usize]);
                }
            }
        }
    }
}

/// `surround_analysis()` (`src/opus_multistream_encoder.c:230`).
///
/// `mem` is `st->window_mem` (`channels*MAX_OVERLAP` entries) and `preemph_mem`
/// is `st->preemph_mem` (`channels` entries); `bandLogE` is `21*channels`
/// entries.
pub fn surround_analysis(
    celt_mode: &CELTMode,
    pcm: &MsEncoderPcm,
    bandLogE: &mut [celt_glog],
    mem: &mut [opus_val32],
    preemph_mem: &mut [opus_val32],
    len: i32,
    overlap: i32,
    channels: i32,
    rate: i32,
    arch: i32,
) {
    let mut pos = [0i32; 8];
    let upsample;
    let frame_size;
    let freq_size;
    let channel_offset;
    let mut bandE = [0.0f32; 21];
    let mut maskLogE = [[0.0f32; 21]; 3];

    upsample = resampling_factor(rate);
    frame_size = len * upsample;

    /* LM = log2(frame_size / 120) */
    let mut LM = 0;
    while LM < celt_mode.maxLM {
        if celt_mode.shortMdctSize << LM == frame_size {
            break;
        }
        LM += 1;
    }

    freq_size = celt_mode.shortMdctSize << LM;

    let mut in_ = vec![0.0 as opus_val32; (frame_size + overlap) as usize];
    let mut x = vec![0.0 as opus_res; len as usize];
    let mut freq = vec![0.0 as opus_val32; freq_size as usize];

    channel_pos(channels, &mut pos);

    for c in 0..3 {
        for i in 0..21 {
            maskLogE[c][i] = -28.0f32;
        }
    }

    for c in 0..channels {
        let nb_frames = frame_size / freq_size;
        celt_assert!(nb_frames * freq_size == frame_size);
        /* OPUS_COPY(in, mem+c*overlap, overlap) */
        for i in 0..overlap {
            in_[i as usize] = mem[(c * overlap + i) as usize];
        }
        pcm.copy_channel_in(&mut x, 1, channels, c, len);
        celt_preemphasis(
            &x,
            &mut in_[overlap as usize..],
            frame_size,
            1,
            upsample,
            &celt_mode.preemph,
            &mut preemph_mem[c as usize],
            0,
        );
        {
            let sum = celt_inner_prod(&in_, &in_, frame_size + overlap);
            /* This should filter out both NaNs and ridiculous signals that could
               cause NaNs further down. */
            if !(sum < 1e18f32) || celt_isnan(sum) {
                for v in in_.iter_mut() {
                    *v = 0.0;
                }
                preemph_mem[c as usize] = 0.0;
            }
        }
        for v in bandE.iter_mut() {
            *v = 0.0;
        }
        for frame in 0..nb_frames {
            let mut tmpE = [0.0 as celt_ener; 21];
            clt_mdct_forward(
                &celt_mode.mdct,
                &in_[(freq_size * frame) as usize..],
                &mut freq,
                &celt_mode.window,
                overlap,
                celt_mode.maxLM - LM,
                1,
                arch,
            );
            if upsample != 1 {
                let bound = freq_size / upsample;
                for i in 0..bound {
                    freq[i as usize] *= upsample as f32;
                }
                for i in bound..freq_size {
                    freq[i as usize] = 0.0;
                }
            }

            compute_band_energies(celt_mode, &freq, &mut tmpE, 21, 1, LM, arch);
            /* If we have multiple frames, take the max energy. */
            for i in 0..21 {
                bandE[i] = MAX32(bandE[i], tmpE[i]);
            }
        }
        amp2Log2(celt_mode, 21, 21, &bandE, &mut bandLogE[(21 * c) as usize..], 1);
        /* Apply spreading function with -6 dB/band going up and -12 dB/band going down. */
        for i in 1..21i32 {
            bandLogE[(21 * c + i) as usize] =
                MAXG(bandLogE[(21 * c + i) as usize], bandLogE[(21 * c + i - 1) as usize] - 1.0f32);
        }
        for i in (0..20i32).rev() {
            bandLogE[(21 * c + i) as usize] =
                MAXG(bandLogE[(21 * c + i) as usize], bandLogE[(21 * c + i + 1) as usize] - 2.0f32);
        }
        if pos[c as usize] == 1 {
            for i in 0..21i32 {
                maskLogE[0][i as usize] =
                    logSum(maskLogE[0][i as usize], bandLogE[(21 * c + i) as usize]);
            }
        } else if pos[c as usize] == 3 {
            for i in 0..21i32 {
                maskLogE[2][i as usize] =
                    logSum(maskLogE[2][i as usize], bandLogE[(21 * c + i) as usize]);
            }
        } else if pos[c as usize] == 2 {
            for i in 0..21i32 {
                maskLogE[0][i as usize] =
                    logSum(maskLogE[0][i as usize], bandLogE[(21 * c + i) as usize] - 0.5f32);
                maskLogE[2][i as usize] =
                    logSum(maskLogE[2][i as usize], bandLogE[(21 * c + i) as usize] - 0.5f32);
            }
        }
        /* OPUS_COPY(mem+c*overlap, in+frame_size, overlap) */
        for i in 0..overlap {
            mem[(c * overlap + i) as usize] = in_[(frame_size + i) as usize];
        }
    }
    for i in 0..21 {
        maskLogE[1][i] = MIN32(maskLogE[0][i], maskLogE[2][i]);
    }
    channel_offset = HALF16(celt_log2(QCONST32(2.0f32, 14) / (channels - 1) as f32));
    for c in 0..3 {
        for i in 0..21 {
            maskLogE[c][i] += channel_offset;
        }
    }
    for c in 0..channels {
        if pos[c as usize] != 0 {
            let mask = &maskLogE[(pos[c as usize] - 1) as usize];
            for i in 0..21i32 {
                bandLogE[(21 * c + i) as usize] -= mask[i as usize];
            }
        } else {
            for i in 0..21i32 {
                bandLogE[(21 * c + i) as usize] = 0.0;
            }
        }
    }
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_encoder_get_size / surround_encoder_get_size
 * ------------------------------------------------------------------------- */

/// `opus_multistream_encoder_get_size()` (`src/opus_multistream_encoder.c:383`).
pub fn opus_multistream_encoder_get_size(nb_streams: i32, nb_coupled_streams: i32) -> i32 {
    if nb_streams < 1 || nb_coupled_streams > nb_streams || nb_coupled_streams < 0 {
        return 0;
    }
    let coupled_size = OpusEncoder::get_size(2);
    let mono_size = OpusEncoder::get_size(1);
    MS_ENCODER_ALIGN_SIZE
        + nb_coupled_streams * align(coupled_size)
        + (nb_streams - nb_coupled_streams) * align(mono_size)
}

/// `opus_multistream_surround_encoder_get_size()` (`src/opus_multistream_encoder.c:396`).
pub fn opus_multistream_surround_encoder_get_size(channels: i32, mapping_family: i32) -> i32 {
    let mut nb_streams = 0;
    let mut nb_coupled_streams = 0;
    let mut size;

    if mapping_family == 0 {
        if channels == 1 {
            nb_streams = 1;
            nb_coupled_streams = 0;
        } else if channels == 2 {
            nb_streams = 1;
            nb_coupled_streams = 1;
        } else {
            return 0;
        }
    } else if mapping_family == 1 && channels <= 8 && channels >= 1 {
        nb_streams = vorbis_mappings[(channels - 1) as usize].nb_streams;
        nb_coupled_streams = vorbis_mappings[(channels - 1) as usize].nb_coupled_streams;
    } else if mapping_family == 255 {
        nb_streams = channels;
        nb_coupled_streams = 0;
    } else if mapping_family == 2 {
        if validate_ambisonics(channels, Some(&mut nb_streams), Some(&mut nb_coupled_streams)) == 0 {
            return 0;
        }
    } else {
        return 0;
    }
    size = opus_multistream_encoder_get_size(nb_streams, nb_coupled_streams);
    if channels > 2 {
        size += channels * (MAX_OVERLAP * 4 + 4);
    }
    size
}

/* ------------------------------------------------------------------------- *
 * init / create
 * ------------------------------------------------------------------------- */

/// Max size in case the encoder decides to return six frames (6 x 20 ms =
/// 120 ms) — `MS_FRAME_TMP` (`src/opus_multistream_encoder.c:838`).
const MS_FRAME_TMP: i32 = 6 * 1275 + 12;

/// `opus_multistream_encoder_init_impl()` (`src/opus_multistream_encoder.c:436`).
///
/// `st == None` is the C `st == NULL` case: validate the arguments and return
/// the allocation size.
fn opus_multistream_encoder_init_impl(
    st: Option<&mut OpusMSEncoder>,
    Fs: opus_int32,
    channels: i32,
    streams: i32,
    coupled_streams: i32,
    mapping: &[u8],
    application: i32,
    mapping_type: MappingType,
) -> i32 {
    if (channels > 255)
        || (channels < 1)
        || (coupled_streams > streams)
        || (streams < 1)
        || (coupled_streams < 0)
        || (streams > 255 - coupled_streams)
        || (streams + coupled_streams > channels)
    {
        return OPUS_BAD_ARG;
    }

    let coupled_size = OpusEncoder::opus_encoder_init_size(Fs, 2, application);
    if coupled_size < 0 {
        return coupled_size;
    }
    let mono_size = OpusEncoder::opus_encoder_init_size(Fs, 1, application);
    if mono_size < 0 {
        return mono_size;
    }

    let st = match st {
        None => {
            let surround_size = if mapping_type == MappingType::Surround {
                channels * (MAX_OVERLAP * 4 + 4)
            } else {
                0
            };
            return MS_ENCODER_ALIGN_SIZE
                + coupled_streams * align(coupled_size)
                + (streams - coupled_streams) * align(mono_size)
                + surround_size;
        }
        Some(s) => s,
    };

    if mapping.len() < channels as usize {
        /* C reads `mapping[0..channels]` unconditionally (a NULL/short mapping
           is undefined behaviour there); the port rejects it. */
        return OPUS_BAD_ARG;
    }

    st.arch = opus_select_arch();
    st.layout.nb_channels = channels;
    st.layout.nb_streams = streams;
    st.layout.nb_coupled_streams = coupled_streams;
    if mapping_type != MappingType::Surround {
        st.lfe_stream = -1;
    }
    st.bitrate_bps = OPUS_AUTO;
    st.application = application;
    st.Fs = Fs;
    st.variable_duration = OPUS_FRAMESIZE_ARG;
    for i in 0..channels as usize {
        st.layout.mapping[i] = mapping[i];
    }
    if validate_layout(&st.layout) == 0 {
        return OPUS_BAD_ARG;
    }
    if validate_encoder_layout(&st.layout) == 0 {
        return OPUS_BAD_ARG;
    }
    if mapping_type == MappingType::Ambisonics
        && validate_ambisonics(st.layout.nb_channels, None, None) == 0
    {
        return OPUS_BAD_ARG;
    }

    st.enc.clear();
    let mut i = 0;
    while i < st.layout.nb_coupled_streams {
        let mut enc = OpusEncoder::default();
        let ret = enc.init(Fs, 2, application);
        if ret != OPUS_OK {
            return ret;
        }
        if i == st.lfe_stream {
            let mut one = 1;
            enc.ctl(OPUS_SET_LFE_REQUEST, &mut one);
        }
        st.enc.push(enc);
        i += 1;
    }
    while i < st.layout.nb_streams {
        let mut enc = OpusEncoder::default();
        let ret = enc.init(Fs, 1, application);
        if ret != OPUS_OK {
            return ret;
        }
        if i == st.lfe_stream {
            let mut one = 1;
            enc.ctl(OPUS_SET_LFE_REQUEST, &mut one);
        }
        st.enc.push(enc);
        i += 1;
    }
    if mapping_type == MappingType::Surround {
        st.window_mem = vec![0.0 as opus_val32; (channels * MAX_OVERLAP) as usize];
        st.preemph_mem = vec![0.0 as opus_val32; channels as usize];
    } else {
        st.window_mem.clear();
        st.preemph_mem.clear();
    }
    st.mapping_type = mapping_type;
    OPUS_OK
}

/// `opus_multistream_encoder_init()` (`src/opus_multistream_encoder.c:519`).
pub fn opus_multistream_encoder_init(
    st: Option<&mut OpusMSEncoder>,
    Fs: opus_int32,
    channels: i32,
    streams: i32,
    coupled_streams: i32,
    mapping: &[u8],
    application: i32,
) -> i32 {
    opus_multistream_encoder_init_impl(
        st,
        Fs,
        channels,
        streams,
        coupled_streams,
        mapping,
        application,
        MappingType::None,
    )
}

/// `opus_multistream_surround_encoder_init()` (`src/opus_multistream_encoder.c:534`).
///
/// `streams`, `coupled_streams` and `mapping` are the C output parameters.
pub fn opus_multistream_surround_encoder_init(
    mut st: Option<&mut OpusMSEncoder>,
    Fs: opus_int32,
    channels: i32,
    mapping_family: i32,
    streams: &mut i32,
    coupled_streams: &mut i32,
    mapping: &mut [u8],
    application: i32,
) -> i32 {
    let mapping_type: MappingType;
    let mut lfe_stream;

    if channels > 255 || channels < 1 {
        return OPUS_BAD_ARG;
    }
    lfe_stream = -1;
    if mapping_family == 0 {
        if channels == 1 {
            *streams = 1;
            *coupled_streams = 0;
            mapping[0] = 0;
        } else if channels == 2 {
            *streams = 1;
            *coupled_streams = 1;
            mapping[0] = 0;
            mapping[1] = 1;
        } else {
            return OPUS_UNIMPLEMENTED;
        }
    } else if mapping_family == 1 && channels <= 8 && channels >= 1 {
        *streams = vorbis_mappings[(channels - 1) as usize].nb_streams;
        *coupled_streams = vorbis_mappings[(channels - 1) as usize].nb_coupled_streams;
        for i in 0..channels as usize {
            mapping[i] = vorbis_mappings[(channels - 1) as usize].mapping[i];
        }
        if channels >= 6 {
            lfe_stream = *streams - 1;
        }
    } else if mapping_family == 255 {
        *streams = channels;
        *coupled_streams = 0;
        for i in 0..channels as usize {
            mapping[i] = i as u8;
        }
    } else if mapping_family == 2 {
        if validate_ambisonics(channels, Some(streams), Some(coupled_streams)) == 0 {
            return OPUS_BAD_ARG;
        }
        for i in 0..(*streams - *coupled_streams) {
            mapping[i as usize] = (i + (*coupled_streams * 2)) as u8;
        }
        for i in 0..(*coupled_streams * 2) {
            mapping[(i + (*streams - *coupled_streams)) as usize] = i as u8;
        }
    } else {
        return OPUS_UNIMPLEMENTED;
    }

    if channels > 2 && mapping_family == 1 {
        mapping_type = MappingType::Surround;
    } else if mapping_family == 2 {
        mapping_type = MappingType::Ambisonics;
    } else {
        mapping_type = MappingType::None;
    }
    if let Some(s) = st.as_mut() {
        s.lfe_stream = lfe_stream;
    }
    opus_multistream_encoder_init_impl(
        st,
        Fs,
        channels,
        *streams,
        *coupled_streams,
        mapping,
        application,
        mapping_type,
    )
}

/// `opus_multistream_encoder_create()` (`src/opus_multistream_encoder.c:611`).
///
/// Rust's `Box` cannot fail, so the C `OPUS_ALLOC_FAIL` return is unreachable.
pub fn opus_multistream_encoder_create(
    Fs: opus_int32,
    channels: i32,
    streams: i32,
    coupled_streams: i32,
    mapping: &[u8],
    application: i32,
    error: &mut i32,
) -> Option<Box<OpusMSEncoder>> {
    if (channels > 255)
        || (channels < 1)
        || (coupled_streams > streams)
        || (streams < 1)
        || (coupled_streams < 0)
        || (streams > 255 - coupled_streams)
        || (streams + coupled_streams > channels)
    {
        *error = OPUS_BAD_ARG;
        return None;
    }
    let mut st = Box::new(OpusMSEncoder::default());
    let ret = opus_multistream_encoder_init(
        Some(&mut st),
        Fs,
        channels,
        streams,
        coupled_streams,
        mapping,
        application,
    );
    *error = ret;
    if ret != OPUS_OK {
        return None;
    }
    Some(st)
}

/// `opus_multistream_surround_encoder_create()` (`src/opus_multistream_encoder.c:657`).
pub fn opus_multistream_surround_encoder_create(
    Fs: opus_int32,
    channels: i32,
    mapping_family: i32,
    streams: &mut i32,
    coupled_streams: &mut i32,
    mapping: &mut [u8],
    application: i32,
    error: &mut i32,
) -> Option<Box<OpusMSEncoder>> {
    if channels > 255 || channels < 1 {
        *error = OPUS_BAD_ARG;
        return None;
    }
    let mut st = Box::new(OpusMSEncoder::default());
    let ret = opus_multistream_surround_encoder_init(
        Some(&mut st),
        Fs,
        channels,
        mapping_family,
        streams,
        coupled_streams,
        mapping,
        application,
    );
    *error = ret;
    if ret != OPUS_OK {
        return None;
    }
    Some(st)
}

/* ------------------------------------------------------------------------- *
 * rate allocation
 * ------------------------------------------------------------------------- */

/// `surround_rate_allocation()` (`src/opus_multistream_encoder.c:702`).
fn surround_rate_allocation(
    st: &OpusMSEncoder,
    rate: &mut [opus_int32],
    frame_size: i32,
    Fs: opus_int32,
) {
    let nb_lfe;
    let nb_uncoupled;
    let nb_coupled;
    let nb_normal;
    let lfe_offset;
    let mut coupled_ratio; /* Q8 */
    let lfe_ratio; /* Q8 */
    let mut channel_offset;
    let mut bitrate;
    let total;
    let mut stream_offset;
    let channel_rate: opus_int32;

    nb_lfe = (st.lfe_stream != -1) as i32;
    nb_coupled = st.layout.nb_coupled_streams;
    nb_uncoupled = st.layout.nb_streams - nb_coupled - nb_lfe;
    nb_normal = 2 * nb_coupled + nb_uncoupled;

    /* Give each non-LFE channel enough bits per channel for coding band energy. */
    channel_offset = 40 * IMAX(50, Fs / frame_size);

    if st.bitrate_bps == OPUS_AUTO {
        bitrate = nb_normal * (channel_offset + Fs + 10000) + 8000 * nb_lfe;
    } else if st.bitrate_bps == OPUS_BITRATE_MAX {
        bitrate = nb_normal * 750000 + nb_lfe * 128000;
    } else {
        bitrate = st.bitrate_bps;
    }

    /* Give LFE some basic stream_channel allocation but never exceed 1/20 of the
       total rate for the non-energy part to avoid problems at really low rate. */
    lfe_offset = IMIN(bitrate / 20, 3000) + 15 * IMAX(50, Fs / frame_size);

    /* We give each stream (coupled or uncoupled) a starting bitrate.
       This models the main saving of coupled channels over uncoupled. */
    stream_offset = (bitrate - channel_offset * nb_normal - lfe_offset * nb_lfe) / nb_normal / 2;
    stream_offset = IMAX(0, IMIN(20000, stream_offset));

    /* Coupled streams get twice the mono rate after the offset is allocated. */
    coupled_ratio = 512;
    /* Should depend on the bitrate, for now we assume LFE gets 1/8 the bits of mono */
    lfe_ratio = 32;

    total = (nb_uncoupled << 8)         /* mono */
          + coupled_ratio * nb_coupled  /* stereo */
          + nb_lfe * lfe_ratio;
    channel_rate = (256i64
        * (bitrate - lfe_offset * nb_lfe
            - stream_offset * (nb_coupled + nb_uncoupled)
            - channel_offset * nb_normal) as i64
        / total as i64) as opus_int32;

    let mut i = 0;
    while i < st.layout.nb_streams {
        if i < st.layout.nb_coupled_streams {
            rate[i as usize] =
                2 * channel_offset + IMAX(0, stream_offset + (channel_rate * coupled_ratio >> 8));
        } else if i != st.lfe_stream {
            rate[i as usize] = channel_offset + IMAX(0, stream_offset + channel_rate);
        } else {
            rate[i as usize] = IMAX(0, lfe_offset + (channel_rate * lfe_ratio >> 8));
        }
        i += 1;
    }
}

/// `ambisonics_rate_allocation()` (`src/opus_multistream_encoder.c:771`).
fn ambisonics_rate_allocation(
    st: &OpusMSEncoder,
    rate: &mut [opus_int32],
    frame_size: i32,
    Fs: opus_int32,
) {
    let total_rate;
    let per_stream_rate;

    let nb_channels = st.layout.nb_streams + st.layout.nb_coupled_streams;

    if st.bitrate_bps == OPUS_AUTO {
        total_rate = (st.layout.nb_coupled_streams + st.layout.nb_streams)
            * (Fs + 60 * Fs / frame_size)
            + st.layout.nb_streams * 15000;
    } else if st.bitrate_bps == OPUS_BITRATE_MAX {
        total_rate = nb_channels * 750000;
    } else {
        total_rate = st.bitrate_bps;
    }

    /* Allocate equal number of bits to Ambisonic (uncoupled) and non-diegetic
     * (coupled) streams */
    per_stream_rate = total_rate / st.layout.nb_streams;
    for i in 0..st.layout.nb_streams {
        rate[i as usize] = per_stream_rate;
    }
}

/// `rate_allocation()` (`src/opus_multistream_encoder.c:805`).
fn rate_allocation(st: &mut OpusMSEncoder, rate: &mut [opus_int32], frame_size: i32) -> opus_int32 {
    let mut rate_sum: opus_int32 = 0;
    let mut Fs = 0i32;

    st.enc[0].ctl(OPUS_GET_SAMPLE_RATE_REQUEST, &mut Fs);

    if st.mapping_type == MappingType::Ambisonics {
        ambisonics_rate_allocation(st, rate, frame_size, Fs);
    } else {
        surround_rate_allocation(st, rate, frame_size, Fs);
    }

    for i in 0..st.layout.nb_streams {
        rate[i as usize] = IMAX(rate[i as usize], 500);
        rate_sum += rate[i as usize];
    }
    rate_sum
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_encode_native
 * ------------------------------------------------------------------------- */

/// `opus_multistream_encode_native()` (`src/opus_multistream_encoder.c:841`).
///
/// `pcm` carries the caller's PCM type (`opus_copy_channel_in_short/int24/float`
/// and its matching `downmix`); `data` must have room for `max_data_bytes`.
pub fn opus_multistream_encode_native(
    st: &mut OpusMSEncoder,
    pcm: &MsEncoderPcm,
    analysis_frame_size: i32,
    data: &mut [u8],
    max_data_bytes: opus_int32,
    lsb_depth: i32,
    float_api: i32,
) -> i32 {
    let mut Fs = 0i32;
    let mut vbr = 0i32;
    let mut frame_size;
    let mut rate_sum;
    let mut smallest_packet;
    let mut max_data_bytes = max_data_bytes;
    let mut bitrates = [0 as opus_int32; 256];
    let mut bandLogE = [0.0 as celt_glog; 42];
    let nb_channels = st.layout.nb_channels;
    let nb_streams = st.layout.nb_streams;
    let nb_coupled_streams = st.layout.nb_coupled_streams;
    let surround = st.mapping_type == MappingType::Surround;
    let ambisonics = st.mapping_type == MappingType::Ambisonics;
    let celt_analysis = surround && st.application != OPUS_APPLICATION_RESTRICTED_SILK;

    st.enc[0].ctl(OPUS_GET_SAMPLE_RATE_REQUEST, &mut Fs);
    st.enc[0].ctl(OPUS_GET_VBR_REQUEST, &mut vbr);

    frame_size = frame_size_select(st.application, analysis_frame_size, st.variable_duration, Fs);
    if frame_size <= 0 {
        return OPUS_BAD_ARG;
    }

    /* Smallest packet the encoder can produce. */
    smallest_packet = nb_streams * 2 - 1;
    /* 100 ms needs an extra byte per stream for the ToC. */
    if Fs / frame_size == 10 {
        smallest_packet += nb_streams;
    }
    if max_data_bytes < smallest_packet {
        return OPUS_BUFFER_TOO_SMALL;
    }

    let mut buf = vec![0.0 as opus_res; (2 * frame_size) as usize];
    let mut bandSMR = vec![0.0 as celt_glog; (21 * nb_channels) as usize];

    if celt_analysis {
        let celt_mode = st.enc[0].get_mode();
        let mut window_mem = std::mem::take(&mut st.window_mem);
        let mut preemph_mem = std::mem::take(&mut st.preemph_mem);
        surround_analysis(
            celt_mode,
            pcm,
            &mut bandSMR,
            &mut window_mem,
            &mut preemph_mem,
            frame_size,
            celt_mode.overlap,
            nb_channels,
            Fs,
            st.arch,
        );
        st.window_mem = window_mem;
        st.preemph_mem = preemph_mem;
    }

    /* Compute bitrate allocation between streams (this could be a lot better) */
    rate_sum = rate_allocation(st, &mut bitrates, frame_size);

    if vbr == 0 {
        if st.bitrate_bps == OPUS_AUTO {
            max_data_bytes = IMIN(
                max_data_bytes,
                (bitrate_to_bits(rate_sum, Fs, frame_size) + 4) / 8,
            );
        } else if st.bitrate_bps != OPUS_BITRATE_MAX {
            max_data_bytes = IMIN(
                max_data_bytes,
                IMAX(
                    smallest_packet,
                    (bitrate_to_bits(st.bitrate_bps, Fs, frame_size) + 4) / 8,
                ),
            );
        }
    }

    for s in 0..nb_streams {
        let enc = &mut st.enc[s as usize];
        let mut rate = bitrates[s as usize];
        enc.ctl(OPUS_SET_BITRATE_REQUEST, &mut rate);
        if surround {
            let mut equiv_rate;
            equiv_rate = st.bitrate_bps;
            if frame_size * 50 < Fs {
                equiv_rate -= 60 * (Fs / frame_size - 50) * nb_channels;
            }
            let mut bw = if equiv_rate > 10000 * nb_channels {
                OPUS_BANDWIDTH_FULLBAND
            } else if equiv_rate > 7000 * nb_channels {
                OPUS_BANDWIDTH_SUPERWIDEBAND
            } else if equiv_rate > 5000 * nb_channels {
                OPUS_BANDWIDTH_WIDEBAND
            } else {
                OPUS_BANDWIDTH_NARROWBAND
            };
            enc.ctl(OPUS_SET_BANDWIDTH_REQUEST, &mut bw);
            if s < nb_coupled_streams {
                /* To preserve the spatial image, force stereo CELT on coupled streams */
                let mut mode = MODE_CELT_ONLY;
                enc.ctl(OPUS_SET_FORCE_MODE_REQUEST, &mut mode);
                let mut two = 2;
                enc.ctl(OPUS_SET_FORCE_CHANNELS_REQUEST, &mut two);
            }
        } else if ambisonics {
            let mut mode = MODE_CELT_ONLY;
            enc.ctl(OPUS_SET_FORCE_MODE_REQUEST, &mut mode);
        }
    }

    /* Counting ToC */
    let mut tot_size: i32 = 0;
    let mut pos: usize = 0;
    let mut tmp_data = [0u8; MS_FRAME_TMP as usize];
    for s in 0..nb_streams {
        let mut rp = OpusRepacketizer::default();
        rp.init();
        let mut c1;
        let mut c2;
        if s < nb_coupled_streams {
            let left = get_left_channel(&st.layout, s, -1);
            let right = get_right_channel(&st.layout, s, -1);
            pcm.copy_channel_in(&mut buf, 2, nb_channels, left, frame_size);
            pcm.copy_channel_in(&mut buf[1..], 2, nb_channels, right, frame_size);
            if celt_analysis {
                for i in 0..21i32 {
                    bandLogE[i as usize] = bandSMR[(21 * left + i) as usize];
                    bandLogE[(21 + i) as usize] = bandSMR[(21 * right + i) as usize];
                }
            }
            c1 = left;
            c2 = right;
        } else {
            let chan = get_mono_channel(&st.layout, s, -1);
            pcm.copy_channel_in(&mut buf, 1, nb_channels, chan, frame_size);
            if celt_analysis {
                for i in 0..21i32 {
                    bandLogE[i as usize] = bandSMR[(21 * chan + i) as usize];
                }
            }
            c1 = chan;
            c2 = -1;
        }
        let enc = &mut st.enc[s as usize];
        if celt_analysis {
            enc.set_energy_mask(Some(&bandLogE));
        }
        /* number of bytes left (+Toc) */
        let mut curr_max = max_data_bytes - tot_size;
        /* Reserve one byte for the last stream and two for the others */
        curr_max -= IMAX(0, 2 * (nb_streams - s - 1) - 1);
        /* For 100 ms, reserve an extra byte per stream for the ToC */
        if Fs / frame_size == 10 {
            curr_max -= nb_streams - s - 1;
        }
        curr_max = IMIN(curr_max, MS_FRAME_TMP);
        /* Repacketizer will add one or two bytes for self-delimited frames */
        if s != nb_streams - 1 {
            curr_max -= if curr_max > 253 { 2 } else { 1 };
        }
        if vbr == 0 && s == nb_streams - 1 {
            let mut v = bits_to_bitrate(curr_max * 8, Fs, frame_size);
            enc.ctl(OPUS_SET_BITRATE_REQUEST, &mut v);
        }
        let len = opus_encode_native(
            enc,
            &buf,
            frame_size,
            &mut tmp_data,
            curr_max,
            lsb_depth,
            Some(pcm.analysis()),
            analysis_frame_size,
            c1,
            c2,
            nb_channels,
            float_api,
        );
        if len < 0 {
            return len;
        }
        /* We need to use the repacketizer to add the self-delimiting lengths
           while taking into account the fact that the encoder can now return
           more than one frame at a time (e.g. 60 ms CELT-only) */
        let ret = rp.cat(&tmp_data[..len as usize], len);
        /* If the opus_repacketizer_cat() fails, then something's seriously wrong
           with the encoder. */
        if ret != OPUS_OK {
            return OPUS_INTERNAL_ERROR;
        }
        let len = rp.opus_repacketizer_out_range_impl(
            0,
            rp.get_nb_frames(),
            &mut data[pos..],
            max_data_bytes - tot_size,
            (s != nb_streams - 1) as i32,
            (vbr == 0 && s == nb_streams - 1) as i32,
            &[],
        );
        if len < 0 {
            /* C would advance the output pointer by a negative length here
               (undefined behaviour); the byte reservation above makes this
               unreachable for every valid input. */
            return len;
        }
        pos += len as usize;
        tot_size += len;
    }
    tot_size
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_encode / encode24 / encode_float
 * ------------------------------------------------------------------------- */

/// `opus_multistream_encode()` (`src/opus_multistream_encoder.c:1111`).
pub fn opus_multistream_encode(
    st: &mut OpusMSEncoder,
    pcm: &[i16],
    frame_size: i32,
    data: &mut [u8],
    max_data_bytes: opus_int32,
) -> i32 {
    opus_multistream_encode_native(
        st,
        &MsEncoderPcm::Short(pcm),
        frame_size,
        data,
        max_data_bytes,
        16,
        0,
    )
}

/// `opus_multistream_encode24()` (`src/opus_multistream_encoder.c:1123`).
pub fn opus_multistream_encode24(
    st: &mut OpusMSEncoder,
    pcm: &[i32],
    frame_size: i32,
    data: &mut [u8],
    max_data_bytes: opus_int32,
) -> i32 {
    opus_multistream_encode_native(
        st,
        &MsEncoderPcm::Int24(pcm),
        frame_size,
        data,
        max_data_bytes,
        MAX_ENCODING_DEPTH,
        0,
    )
}

/// `opus_multistream_encode_float()` (`src/opus_multistream_encoder.c:1136`).
pub fn opus_multistream_encode_float(
    st: &mut OpusMSEncoder,
    pcm: &[f32],
    frame_size: i32,
    data: &mut [u8],
    max_data_bytes: opus_int32,
) -> i32 {
    opus_multistream_encode_native(
        st,
        &MsEncoderPcm::Float(pcm),
        frame_size,
        data,
        max_data_bytes,
        MAX_ENCODING_DEPTH,
        1,
    )
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_encoder_ctl / destroy
 * ------------------------------------------------------------------------- */

impl OpusMSEncoder {
    /// `OPUS_MULTISTREAM_GET_ENCODER_STATE_REQUEST` — the typed equivalent of
    /// the C `opus_multistream_encoder_ctl(st, OPUS_MULTISTREAM_GET_ENCODER_STATE(stream_id), &enc)`.
    ///
    /// Returns `Err(OPUS_BAD_ARG)` when `stream_id` is out of range, exactly
    /// like C (which `goto`s `bad_arg`).
    pub fn get_encoder_state(&mut self, stream_id: i32) -> Result<&mut OpusEncoder, i32> {
        if stream_id < 0 || stream_id >= self.layout.nb_streams {
            return Err(OPUS_BAD_ARG);
        }
        Ok(&mut self.enc[stream_id as usize])
    }
}

/// `opus_multistream_encoder_ctl_va_list()` +
/// `opus_multistream_encoder_ctl()` (`src/opus_multistream_encoder.c:1149`).
///
/// `value` is an in/out integer (`docs/internal-api.md`).  Requests whose C
/// argument is a pointer (`OPUS_MULTISTREAM_GET_ENCODER_STATE`) go through
/// [`OpusMSEncoder::get_encoder_state`] and report `OPUS_UNIMPLEMENTED` here.
pub fn opus_multistream_encoder_ctl(st: &mut OpusMSEncoder, request: i32, value: &mut i32) -> i32 {
    let mut ret = OPUS_OK;

    match request {
        OPUS_SET_BITRATE_REQUEST => {
            let mut v = *value;
            if v != OPUS_AUTO && v != OPUS_BITRATE_MAX {
                if v <= 0 {
                    return OPUS_BAD_ARG;
                }
                v = IMIN(750000 * st.layout.nb_channels, IMAX(500 * st.layout.nb_channels, v));
            }
            st.bitrate_bps = v;
        }
        OPUS_GET_BITRATE_REQUEST => {
            *value = 0;
            for s in 0..st.layout.nb_streams {
                let mut rate = 0i32;
                opus_encoder_ctl_forward(&mut st.enc[s as usize], request, &mut rate);
                *value += rate;
            }
        }
        OPUS_GET_LSB_DEPTH_REQUEST
        | OPUS_GET_VBR_REQUEST
        | OPUS_GET_APPLICATION_REQUEST
        | OPUS_GET_BANDWIDTH_REQUEST
        | OPUS_GET_COMPLEXITY_REQUEST
        | OPUS_GET_PACKET_LOSS_PERC_REQUEST
        | OPUS_GET_DTX_REQUEST
        | OPUS_GET_VOICE_RATIO_REQUEST
        | OPUS_GET_VBR_CONSTRAINT_REQUEST
        | OPUS_GET_SIGNAL_REQUEST
        | OPUS_GET_LOOKAHEAD_REQUEST
        | OPUS_GET_SAMPLE_RATE_REQUEST
        | OPUS_GET_INBAND_FEC_REQUEST
        | OPUS_GET_FORCE_CHANNELS_REQUEST
        | OPUS_GET_PREDICTION_DISABLED_REQUEST
        | OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST
        | OPUS_GET_QEXT_REQUEST => {
            /* For int32* GET params, just query the first stream */
            ret = opus_encoder_ctl_forward(&mut st.enc[0], request, value);
        }
        OPUS_GET_FINAL_RANGE_REQUEST => {
            let mut acc: u32 = 0;
            for s in 0..st.layout.nb_streams {
                let mut tmp = 0i32;
                ret = opus_encoder_ctl_forward(&mut st.enc[s as usize], request, &mut tmp);
                if ret != OPUS_OK {
                    break;
                }
                acc ^= tmp as u32;
            }
            *value = acc as i32;
        }
        OPUS_SET_LSB_DEPTH_REQUEST
        | OPUS_SET_COMPLEXITY_REQUEST
        | OPUS_SET_VBR_REQUEST
        | OPUS_SET_VBR_CONSTRAINT_REQUEST
        | OPUS_SET_MAX_BANDWIDTH_REQUEST
        | OPUS_SET_BANDWIDTH_REQUEST
        | OPUS_SET_SIGNAL_REQUEST
        | OPUS_SET_APPLICATION_REQUEST
        | OPUS_SET_INBAND_FEC_REQUEST
        | OPUS_SET_PACKET_LOSS_PERC_REQUEST
        | OPUS_SET_DTX_REQUEST
        | OPUS_SET_FORCE_MODE_REQUEST
        | OPUS_SET_FORCE_CHANNELS_REQUEST
        | OPUS_SET_PREDICTION_DISABLED_REQUEST
        | OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST
        | OPUS_SET_QEXT_REQUEST => {
            /* This works for int32 params */
            let mut v = *value;
            for s in 0..st.layout.nb_streams {
                ret = opus_encoder_ctl_forward(&mut st.enc[s as usize], request, &mut v);
                if ret != OPUS_OK {
                    break;
                }
            }
        }
        OPUS_SET_EXPERT_FRAME_DURATION_REQUEST => {
            st.variable_duration = *value;
        }
        OPUS_GET_EXPERT_FRAME_DURATION_REQUEST => {
            *value = st.variable_duration;
        }
        OPUS_RESET_STATE => {
            if st.mapping_type == MappingType::Surround {
                for v in st.preemph_mem.iter_mut() {
                    *v = 0.0;
                }
                for v in st.window_mem.iter_mut() {
                    *v = 0.0;
                }
            }
            for s in 0..st.layout.nb_streams {
                let mut dummy = 0i32;
                ret = opus_encoder_ctl_forward(&mut st.enc[s as usize], OPUS_RESET_STATE, &mut dummy);
                if ret != OPUS_OK {
                    break;
                }
            }
        }
        _ => {
            ret = OPUS_UNIMPLEMENTED;
        }
    }
    ret
}

/// `opus_encoder_ctl(enc, request, value)` — the scalar subset
/// `src/opus_multistream_encoder.c` forwards.
#[inline]
fn opus_encoder_ctl_forward(enc: &mut OpusEncoder, request: i32, value: &mut i32) -> i32 {
    enc.ctl(request, value)
}

/// `opus_multistream_encoder_destroy()` (`src/opus_multistream_encoder.c:1360`):
/// the owned `Box` makes this a no-op, kept for contract parity.
pub fn opus_multistream_encoder_destroy(_st: Box<OpusMSEncoder>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opus::multistream_decoder::OpusMSDecoder;

    #[test]
    fn test_vorbis_mappings() {
        let expected: [(i32, i32, [u8; 8]); 8] = [
            (1, 0, [0, 0, 0, 0, 0, 0, 0, 0]),
            (1, 1, [0, 1, 0, 0, 0, 0, 0, 0]),
            (2, 1, [0, 2, 1, 0, 0, 0, 0, 0]),
            (2, 2, [0, 1, 2, 3, 0, 0, 0, 0]),
            (3, 2, [0, 4, 1, 2, 3, 0, 0, 0]),
            (4, 2, [0, 4, 1, 2, 3, 5, 0, 0]),
            (4, 3, [0, 4, 1, 2, 3, 5, 6, 0]),
            (5, 3, [0, 6, 1, 2, 3, 4, 5, 7]),
        ];
        for (i, (s, c, m)) in expected.iter().enumerate() {
            assert_eq!(vorbis_mappings[i].nb_streams, *s);
            assert_eq!(vorbis_mappings[i].nb_coupled_streams, *c);
            assert_eq!(vorbis_mappings[i].mapping, *m);
        }
    }

    #[test]
    fn test_validate_ambisonics() {
        let mut s = 0;
        let mut c = 0;
        /* 4ch: order 2 -> 4 ACN channels, no non-diegetic. */
        assert_eq!(validate_ambisonics(4, Some(&mut s), Some(&mut c)), 1);
        assert_eq!((s, c), (4, 0));
        /* 6ch: 4 ACN + 2 non-diegetic -> 5 streams, 1 coupled. */
        assert_eq!(validate_ambisonics(6, Some(&mut s), Some(&mut c)), 1);
        assert_eq!((s, c), (5, 1));
        /* 3ch: order 1 (1 ACN) + 2 non-diegetic -> 2 streams, 1 coupled. */
        assert_eq!(validate_ambisonics(3, Some(&mut s), Some(&mut c)), 1);
        assert_eq!((s, c), (2, 1));
        /* 1ch: 1 ACN, no non-diegetic. */
        assert_eq!(validate_ambisonics(1, Some(&mut s), Some(&mut c)), 1);
        assert_eq!((s, c), (1, 0));
        /* A `NULL` out-parameter is fine (the C `validate_ambisonics(x, NULL, NULL)` idiom). */
        assert_eq!(validate_ambisonics(4, None, None), 1);
        /* 2/5/7/8 channels are not a valid ambisonic layout. */
        assert_eq!(validate_ambisonics(2, None, None), 0);
        assert_eq!(validate_ambisonics(5, None, None), 0);
        assert_eq!(validate_ambisonics(7, None, None), 0);
        assert_eq!(validate_ambisonics(8, None, None), 0);
        assert_eq!(validate_ambisonics(0, None, None), 0);
        assert_eq!(validate_ambisonics(228, None, None), 0);
    }

    #[test]
    fn test_encoder_get_size() {
        assert_eq!(opus_multistream_encoder_get_size(1, 0), 109408);
        assert_eq!(opus_multistream_encoder_get_size(2, 0), 218520);
        assert_eq!(opus_multistream_encoder_get_size(2, 1), 235536);
        assert_eq!(opus_multistream_encoder_get_size(4, 2), 470776);
        assert_eq!(opus_multistream_encoder_get_size(5, 3), 596904);
        /* Invalid counts return 0, like the C. */
        assert_eq!(opus_multistream_encoder_get_size(0, 0), 0);
        assert_eq!(opus_multistream_encoder_get_size(1, 2), 0);
        assert_eq!(opus_multistream_encoder_get_size(2, -1), 0);
    }

    #[test]
    fn test_decoder_get_size() {
        assert_eq!(OpusMSDecoder::get_size(1, 0), 191856);
        assert_eq!(OpusMSDecoder::get_size(2, 0), 383440);
        assert_eq!(OpusMSDecoder::get_size(2, 1), 392208);
        assert_eq!(OpusMSDecoder::get_size(4, 2), 784144);
        assert_eq!(OpusMSDecoder::get_size(0, 0), 0);
        assert_eq!(OpusMSDecoder::get_size(1, 2), 0);
    }

    #[test]
    fn test_surround_encoder_get_size() {
        assert_eq!(opus_multistream_surround_encoder_get_size(1, 0), 109408);
        assert_eq!(opus_multistream_surround_encoder_get_size(2, 0), 126424);
        assert_eq!(opus_multistream_surround_encoder_get_size(3, 1), 236988);
        assert_eq!(opus_multistream_surround_encoder_get_size(4, 1), 254488);
        assert_eq!(opus_multistream_surround_encoder_get_size(5, 1), 364084);
        assert_eq!(opus_multistream_surround_encoder_get_size(6, 1), 473680);
        assert_eq!(opus_multistream_surround_encoder_get_size(7, 1), 491180);
        assert_eq!(opus_multistream_surround_encoder_get_size(8, 1), 600776);
        /* family 0 only supports mono/stereo. */
        assert_eq!(opus_multistream_surround_encoder_get_size(3, 0), 0);
        assert_eq!(opus_multistream_surround_encoder_get_size(6, 0), 0);
        /* family 255 is one mono stream per channel. */
        assert_eq!(
            opus_multistream_surround_encoder_get_size(4, 255),
            opus_multistream_encoder_get_size(4, 0) + 4 * (MAX_OVERLAP * 4 + 4)
        );
        /* family 2 ambisonics: 4ch -> 4 streams, plus the >2-channel scratch. */
        assert_eq!(
            opus_multistream_surround_encoder_get_size(4, 2),
            opus_multistream_encoder_get_size(4, 0) + 4 * (MAX_OVERLAP * 4 + 4)
        );
        assert_eq!(opus_multistream_surround_encoder_get_size(5, 2), 0);
    }

    #[test]
    fn test_declared_struct_sizes() {
        assert_eq!(MS_ENCODER_ALIGN_SIZE, 296);
        assert_eq!(crate::opus::multistream::MS_DECODER_ALIGN_SIZE, 272);
        assert_eq!(crate::opus::multistream::MS_DECODER_STRUCT_SIZE, 268);
    }

    #[test]
    fn test_create_error_paths() {
        let mut e = 0;
        /* channels < streams+coupled */
        assert!(opus_multistream_encoder_create(48000, 1, 2, 0, &[0], 2049, &mut e).is_none());
        assert_eq!(e, OPUS_BAD_ARG);
        /* coupled > streams */
        assert!(opus_multistream_encoder_create(48000, 3, 1, 2, &[0, 1, 2], 2049, &mut e).is_none());
        assert_eq!(e, OPUS_BAD_ARG);
        /* mapping entries out of range */
        assert!(opus_multistream_encoder_create(48000, 3, 2, 1, &[0, 3, 1], 2049, &mut e).is_none());
        assert_eq!(e, OPUS_BAD_ARG);
        /* a coupled stream whose right channel is missing */
        assert!(opus_multistream_encoder_create(48000, 3, 2, 1, &[0, 0, 1], 2049, &mut e).is_none());
        assert_eq!(e, OPUS_BAD_ARG);
        /* bad sample rate */
        assert!(opus_multistream_encoder_create(44100, 2, 1, 1, &[0, 1], 2049, &mut e).is_none());
        assert_eq!(e, OPUS_BAD_ARG);
        /* bad application */
        assert!(opus_multistream_encoder_create(48000, 2, 1, 1, &[0, 1], 123, &mut e).is_none());
        assert_eq!(e, OPUS_BAD_ARG);
        /* surround family 0 with > 2 channels is unimplemented */
        let mut streams = 0;
        let mut coupled = 0;
        let mut mapping = [0u8; 8];
        assert!(
            opus_multistream_surround_encoder_create(
                48000, 3, 0, &mut streams, &mut coupled, &mut mapping, 2049, &mut e
            )
            .is_none()
        );
        assert_eq!(e, OPUS_UNIMPLEMENTED);
        /* surround family 99 is unimplemented */
        assert!(
            opus_multistream_surround_encoder_create(
                48000, 2, 99, &mut streams, &mut coupled, &mut mapping, 2049, &mut e
            )
            .is_none()
        );
        assert_eq!(e, OPUS_UNIMPLEMENTED);
    }

    #[test]
    fn test_roundtrip_stereo() {
        let mut e = 0;
        let mut enc = opus_multistream_encoder_create(48000, 2, 1, 1, &[0, 1], 2049, &mut e)
            .expect("stereo multistream encoder");
        assert_eq!(e, OPUS_OK);

        let frame = 960;
        let mut pcm = vec![0f32; frame * 2];
        let mut state = 0x1234567u32;
        for i in 0..frame * 2 {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            pcm[i] = (((state >> 8) as i32) - 8388608) as f32 / 8388608.0;
        }
        let mut pkt = vec![0u8; 1500];
        let n = opus_multistream_encode_float(&mut enc, &pcm, frame as i32, &mut pkt, 1500);
        assert!(n > 0, "encode returned {n}");

        let mut dec = OpusMSDecoder::create(48000, 2, 1, 1, &[0, 1], &mut e).expect("decoder");
        let mut out = vec![0i16; frame * 2];
        let ret = dec.decode(Some(&pkt[..n as usize]), n, &mut out, frame as i32, 0);
        assert_eq!(ret, frame as i32);
        assert!(out.iter().any(|&s| s != 0));
    }

    #[test]
    fn test_encode_native_rejects_small_buffer() {
        let mut e = 0;
        let mut enc = opus_multistream_encoder_create(48000, 2, 1, 1, &[0, 1], 2049, &mut e).unwrap();
        let frame = 960;
        let pcm = vec![0f32; frame * 2];
        let mut pkt = vec![0u8; 1500];
        /* `smallest_packet` is `streams*2-1 == 1`; 0 is too small. */
        let n = opus_multistream_encode_float(&mut enc, &pcm, frame as i32, &mut pkt, 0);
        assert_eq!(n, OPUS_BUFFER_TOO_SMALL);
    }
}

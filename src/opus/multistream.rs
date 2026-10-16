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
//! Transliteration of `src/opus_multistream.c` from libopus 1.6.1 into safe
//! Rust: the shared `ChannelLayout` helpers plus the `OpusMSEncoder` /
//! `OpusMSDecoder` state structures that `src/opus_multistream_encoder.c` and
//! `src/opus_multistream_decoder.c` (ported as [`super::multistream_encoder`]
//! and [`super::multistream_decoder`]) operate on.
//!
//! # State layout
//!
//! C lays the per-stream `OpusEncoder` / `OpusDecoder` states out in one raw
//! allocation (`ptr = (char*)st + align(sizeof(OpusMSEncoder))`, with the
//! `nb_coupled_streams` stereo states followed by the mono ones), and appends
//! the surround-analysis scratch (`window_mem[channels*MAX_OVERLAP]`,
//! `preemph_mem[channels]`) after them.  The port owns the states as
//! `Vec<OpusEncoder>` / `Vec<OpusDecoder>` and the scratch as
//! `Vec<opus_val32>`; the C *offsets* are never needed, but the C *sizes* are,
//! because `opus_multistream_encoder_get_size()` reports them to the caller.
//!
//! [`MS_ENCODER_STRUCT_SIZE`] / [`MS_DECODER_STRUCT_SIZE`] are the reference
//! `sizeof(struct OpusMSEncoder)` / `sizeof(struct OpusMSDecoder)` (296 / 268
//! on x86-64) and are validated against the reference library by
//! `examples/probe_multistream.rs`.

use crate::celt::arch::opus_val32;
use crate::opus::decoder::OpusDecoder;
use crate::opus::encoder::OpusEncoder;

/// `MAX_OVERLAP` (`src/opus_multistream_encoder.c:65`; `ENABLE_QEXT` is
/// undefined in this configuration, so the 240 variant is not compiled).
pub const MAX_OVERLAP: i32 = 120;

/// `sizeof(struct OpusMSEncoder)` on x86-64.
pub const MS_ENCODER_STRUCT_SIZE: i32 = 296;
/// `align(sizeof(struct OpusMSEncoder))` — the offset at which C places the
/// per-stream encoder states (equal to the unaligned size here).
pub const MS_ENCODER_ALIGN_SIZE: i32 = 296;
/// `sizeof(struct OpusMSDecoder)` on x86-64.
pub const MS_DECODER_STRUCT_SIZE: i32 = 268;
/// `align(sizeof(struct OpusMSDecoder))` — the offset at which C places the
/// per-stream decoder states.
pub const MS_DECODER_ALIGN_SIZE: i32 = 272;

/// `OPUS_MULTISTREAM_GET_ENCODER_STATE_REQUEST` (`include/opus_multistream.h`).
pub const OPUS_MULTISTREAM_GET_ENCODER_STATE_REQUEST: i32 = 5120;
/// `OPUS_MULTISTREAM_GET_DECODER_STATE_REQUEST` (`include/opus_multistream.h`).
pub const OPUS_MULTISTREAM_GET_DECODER_STATE_REQUEST: i32 = 5122;

/// `align()` (`src/opus_private.h:197`): rounds up to the alignment of a
/// `union { void*; opus_int32; opus_val32; }`, which is 8 on x86-64.
#[inline(always)]
pub fn align(i: i32) -> i32 {
    ((i + 7) / 8) * 8
}

/// `MappingType` (`src/opus_private.h:93`).
///
/// C declares `MAPPING_TYPE_NONE` first, so the Rust `Default` (0) is `None`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MappingType {
    /// `MAPPING_TYPE_NONE`
    #[default]
    None,
    /// `MAPPING_TYPE_SURROUND`
    Surround,
    /// `MAPPING_TYPE_AMBISONICS`
    Ambisonics,
}

/// `ChannelLayout` (`src/opus_private.h:86`).
pub struct ChannelLayout {
    /// `int nb_channels`
    pub nb_channels: i32,
    /// `int nb_streams`
    pub nb_streams: i32,
    /// `int nb_coupled_streams`
    pub nb_coupled_streams: i32,
    /// `unsigned char mapping[256]`
    pub mapping: [u8; 256],
}

impl Default for ChannelLayout {
    fn default() -> Self {
        ChannelLayout {
            nb_channels: 0,
            nb_streams: 0,
            nb_coupled_streams: 0,
            mapping: [0; 256],
        }
    }
}

impl ChannelLayout {
    /// Copies a mapping table in (the caller has already stored the
    /// stream/coupled counts).  C writes the whole array; only the first
    /// `nb_channels` entries are ever read (`validate_layout()` and the
    /// `get_*_channel()` helpers).
    pub fn set_mapping(&mut self, mapping: &[u8]) {
        for (dst, src) in self.mapping.iter_mut().zip(mapping.iter()) {
            *dst = *src;
        }
    }
}

/// `validate_layout()` (`src/opus_multistream.c:38`).
pub fn validate_layout(layout: &ChannelLayout) -> i32 {
    let max_channel;

    max_channel = layout.nb_streams + layout.nb_coupled_streams;
    if max_channel > 255 {
        return 0;
    }
    for i in 0..layout.nb_channels as usize {
        if layout.mapping[i] as i32 >= max_channel && layout.mapping[i] != 255 {
            return 0;
        }
    }
    1
}

/// `get_left_channel()` (`src/opus_multistream.c:54`).
pub fn get_left_channel(layout: &ChannelLayout, stream_id: i32, prev: i32) -> i32 {
    let mut i = if prev < 0 { 0 } else { prev + 1 };
    while i < layout.nb_channels {
        if layout.mapping[i as usize] as i32 == stream_id * 2 {
            return i;
        }
        i += 1;
    }
    -1
}

/// `get_right_channel()` (`src/opus_multistream.c:67`).
pub fn get_right_channel(layout: &ChannelLayout, stream_id: i32, prev: i32) -> i32 {
    let mut i = if prev < 0 { 0 } else { prev + 1 };
    while i < layout.nb_channels {
        if layout.mapping[i as usize] as i32 == stream_id * 2 + 1 {
            return i;
        }
        i += 1;
    }
    -1
}

/// `get_mono_channel()` (`src/opus_multistream.c:80`).
pub fn get_mono_channel(layout: &ChannelLayout, stream_id: i32, prev: i32) -> i32 {
    let mut i = if prev < 0 { 0 } else { prev + 1 };
    while i < layout.nb_channels {
        if layout.mapping[i as usize] as i32 == stream_id + layout.nb_coupled_streams {
            return i;
        }
        i += 1;
    }
    -1
}

/// `struct OpusMSEncoder` (`src/opus_private.h:99`).
///
/// The trailing `window_mem` / `preemph_mem` scratch that C allocates after the
/// encoder states (see `ms_get_window_mem()` / `ms_get_preemph_mem()`) is owned
/// here; the `Vec` lengths are `channels*MAX_OVERLAP` and `channels`
/// respectively, matching the C allocation exactly.
pub struct OpusMSEncoder {
    /// `ChannelLayout layout`
    pub layout: ChannelLayout,
    /// `int arch`
    pub arch: i32,
    /// `int lfe_stream`
    pub lfe_stream: i32,
    /// `int application`
    pub application: i32,
    /// `opus_int32 Fs`
    pub Fs: i32,
    /// `int variable_duration`
    pub variable_duration: i32,
    /// `MappingType mapping_type`
    pub mapping_type: MappingType,
    /// `opus_int32 bitrate_bps`
    pub bitrate_bps: i32,
    /* The encoder states C places in the same allocation. */
    /// One `OpusEncoder` per stream: `nb_coupled_streams` stereo states followed
    /// by `nb_streams-nb_coupled_streams` mono states.
    pub enc: Vec<OpusEncoder>,
    /// `opus_val32 window_mem[channels*MAX_OVERLAP]`
    pub window_mem: Vec<opus_val32>,
    /// `opus_val32 preemph_mem[channels]`
    pub preemph_mem: Vec<opus_val32>,
}

impl Default for OpusMSEncoder {
    fn default() -> Self {
        OpusMSEncoder {
            layout: ChannelLayout::default(),
            arch: 0,
            lfe_stream: 0,
            application: 0,
            Fs: 0,
            variable_duration: 0,
            mapping_type: MappingType::None,
            bitrate_bps: 0,
            enc: Vec::new(),
            window_mem: Vec::new(),
            preemph_mem: Vec::new(),
        }
    }
}

/// `struct OpusMSDecoder` (`src/opus_private.h:113`).
pub struct OpusMSDecoder {
    /// `ChannelLayout layout`
    pub layout: ChannelLayout,
    /// One `OpusDecoder` per stream: `nb_coupled_streams` stereo states followed
    /// by `nb_streams-nb_coupled_streams` mono states.
    pub dec: Vec<OpusDecoder>,
}

impl Default for OpusMSDecoder {
    fn default() -> Self {
        OpusMSDecoder {
            layout: ChannelLayout::default(),
            dec: Vec::new(),
        }
    }
}

/* The `src/opus_multistream_encoder.c` / `src/opus_multistream_decoder.c`
   entry points are re-exported here so that
   `oprs::opus::multistream::opus_multistream_*()` names every multistream API
   from the module that owns the state types. */
pub use super::multistream_decoder::{
    opus_multistream_decode, opus_multistream_decode24, opus_multistream_decode_float,
    opus_multistream_decode_native, opus_multistream_decoder_create,
    opus_multistream_decoder_ctl, opus_multistream_decoder_destroy,
    opus_multistream_decoder_get_size, opus_multistream_decoder_init,
};
pub use super::multistream_encoder::{
    opus_multistream_encode, opus_multistream_encode24, opus_multistream_encode_float,
    opus_multistream_encode_native, opus_multistream_encoder_create,
    opus_multistream_encoder_ctl, opus_multistream_encoder_destroy,
    opus_multistream_encoder_get_size, opus_multistream_encoder_init,
    opus_multistream_surround_encoder_create, opus_multistream_surround_encoder_get_size,
    opus_multistream_surround_encoder_init,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(channels: i32, streams: i32, coupled: i32, mapping: &[u8]) -> ChannelLayout {
        let mut l = ChannelLayout {
            nb_channels: channels,
            nb_streams: streams,
            nb_coupled_streams: coupled,
            ..ChannelLayout::default()
        };
        l.set_mapping(mapping);
        l
    }

    #[test]
    fn test_validate_layout() {
        /* 3 channels, 2 streams, 1 coupled: mapping {0,2,1} is the Vorbis 3ch. */
        assert_eq!(validate_layout(&layout(3, 2, 1, &[0, 2, 1])), 1);
        /* A mapping entry equal to max_channel is rejected. */
        assert_eq!(validate_layout(&layout(3, 2, 1, &[0, 3, 1])), 0);
        /* 255 marks a muted channel and is always accepted. */
        assert_eq!(validate_layout(&layout(3, 2, 1, &[0, 255, 1])), 1);
        /* nb_streams+nb_coupled_streams > 255 is rejected. */
        assert_eq!(validate_layout(&layout(1, 200, 100, &[0])), 0);
    }

    #[test]
    fn test_channel_searches() {
        /* 6-channel 5.1 Vorbis mapping: {0,4,1,2,3,5}. */
        let l = layout(6, 4, 2, &[0, 4, 1, 2, 3, 5]);
        assert_eq!(get_left_channel(&l, 0, -1), 0);
        assert_eq!(get_right_channel(&l, 0, -1), 2);
        assert_eq!(get_left_channel(&l, 0, 0), -1);
        assert_eq!(get_right_channel(&l, 0, 2), -1);
        /* Stream 1 is coupled: left id 2 at index 3, right id 3 at index 4. */
        assert_eq!(get_left_channel(&l, 1, -1), 3);
        assert_eq!(get_right_channel(&l, 1, -1), 4);
        /* Streams 2 and 3 are mono (mapping 4 and 5 => id 2+2 and 3+2). */
        assert_eq!(get_mono_channel(&l, 2, -1), 1);
        assert_eq!(get_mono_channel(&l, 3, -1), 5);
        /* A stream id that maps to no channel at all (the C only calls this for
           `s >= nb_coupled_streams`, so the search is a plain table scan). */
        assert_eq!(get_mono_channel(&l, 9, -1), -1);
    }

    #[test]
    fn test_muted_and_gaps() {
        /* A stream mapped to no channel: `get_mono_channel` finds nothing. */
        let l = layout(3, 3, 0, &[0, 1, 255]);
        assert_eq!(get_mono_channel(&l, 2, -1), -1);
        assert_eq!(get_mono_channel(&l, 1, -1), 1);
        assert_eq!(validate_layout(&l), 1);
    }
}

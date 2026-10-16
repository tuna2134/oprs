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
//! Transliteration of `src/opus_multistream_decoder.c` from libopus 1.6.1 into
//! safe Rust.
//!
//! The per-stream `OpusDecoder` states that C lays out inside the
//! `OpusMSDecoder` allocation are owned by [`OpusMSDecoder::dec`] (see
//! [`super::multistream`]); everything else follows the C one-to-one, including
//! `opus_multistream_packet_validate()` and the `ctl` switch.
//!
//! The C `opus_multistream_decoder_ctl_va_list()` variadic body is exposed as
//! the scalar [`OpusMSDecoder::ctl`] (`value` is an in/out integer, exactly like
//! [`crate::opus::decoder::OpusDecoder::ctl`]).  The one request whose C
//! argument is a pointer, `OPUS_MULTISTREAM_GET_DECODER_STATE`, is provided as
//! the typed [`OpusMSDecoder::get_decoder_state`]; `ctl()` reports
//! `OPUS_UNIMPLEMENTED` for it (C would read a pointer out of the `va_list`,
//! which the scalar interface cannot express).

use crate::celt::arch::{opus_res, RES2FLOAT, RES2INT16, RES2INT24};
use crate::ffi::*;
use crate::opus::decoder::{opus_decode_native, OpusDecoder};
use crate::opus::multistream::{
    align, get_left_channel, get_mono_channel, get_right_channel, validate_layout,
    MS_DECODER_ALIGN_SIZE,
};
use crate::opus::packet::{opus_packet_get_nb_samples, opus_packet_parse_impl};
use crate::types::{opus_int32, opus_uint32};

/// `struct OpusMSDecoder` lives in [`super::multistream`]; re-exported here so
/// that the decoder entry points and their state type share one module path.
pub use crate::opus::multistream::OpusMSDecoder;

/* ------------------------------------------------------------------------- *
 * Output-PCM views (`opus_copy_channel_out_short/int24/float`)
 * ------------------------------------------------------------------------- */

/// The `void *pcm` plus `opus_copy_channel_out_func` pair of
/// `opus_multistream_decode_native()`.
pub enum MsDecoderPcm<'a> {
    /// `opus_copy_channel_out_short()` (`opus_multistream_decode`)
    Short(&'a mut [i16]),
    /// `opus_copy_channel_out_int24()` (`opus_multistream_decode24`)
    Int24(&'a mut [i32]),
    /// `opus_copy_channel_out_float()` (`opus_multistream_decode_float`)
    Float(&'a mut [f32]),
}

impl<'a> MsDecoderPcm<'a> {
    fn write(
        &mut self,
        dst_stride: i32,
        dst_channel: i32,
        src: Option<&[opus_res]>,
        src_stride: i32,
        frame_size: i32,
    ) {
        match self {
            MsDecoderPcm::Short(dst) => {
                if let Some(src) = src {
                    for i in 0..frame_size {
                        dst[(i * dst_stride + dst_channel) as usize] =
                            RES2INT16(src[(i * src_stride) as usize]);
                    }
                } else {
                    for i in 0..frame_size {
                        dst[(i * dst_stride + dst_channel) as usize] = 0;
                    }
                }
            }
            MsDecoderPcm::Int24(dst) => {
                if let Some(src) = src {
                    for i in 0..frame_size {
                        dst[(i * dst_stride + dst_channel) as usize] =
                            RES2INT24(src[(i * src_stride) as usize]);
                    }
                } else {
                    for i in 0..frame_size {
                        dst[(i * dst_stride + dst_channel) as usize] = 0;
                    }
                }
            }
            MsDecoderPcm::Float(dst) => {
                if let Some(src) = src {
                    for i in 0..frame_size {
                        dst[(i * dst_stride + dst_channel) as usize] =
                            RES2FLOAT(src[(i * src_stride) as usize]);
                    }
                } else {
                    for i in 0..frame_size {
                        dst[(i * dst_stride + dst_channel) as usize] = 0.0;
                    }
                }
            }
        }
    }
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_decoder_get_size / init / create
 * ------------------------------------------------------------------------- */

impl OpusMSDecoder {
    /// `opus_multistream_decoder_get_size()` (`src/opus_multistream_decoder.c:59`).
    ///
    /// The per-stream terms use [`OpusDecoder::get_size`], which is pinned to
    /// the reference layout (this is deliberate: the system libopus build drops
    /// the `ENABLE_OSCE_BWE` fields and reports smaller values).
    pub fn get_size(nb_streams: i32, nb_coupled_streams: i32) -> i32 {
        if nb_streams < 1 || nb_coupled_streams > nb_streams || nb_coupled_streams < 0 {
            return 0;
        }
        let coupled_size = OpusDecoder::get_size(2);
        let mono_size = OpusDecoder::get_size(1);
        MS_DECODER_ALIGN_SIZE
            + nb_coupled_streams * align(coupled_size)
            + (nb_streams - nb_coupled_streams) * align(mono_size)
    }

    /// `opus_multistream_decoder_init()` (`src/opus_multistream_decoder.c:71`).
    ///
    /// `mapping` must hold `channels` entries.  C reads them without a NULL
    /// check (a NULL/short mapping is undefined behaviour there); the port
    /// rejects a short mapping with `OPUS_BAD_ARG` instead.
    pub fn init(
        &mut self,
        Fs: opus_int32,
        channels: i32,
        streams: i32,
        coupled_streams: i32,
        mapping: &[u8],
    ) -> i32 {
        if (channels > 255)
            || (channels < 1)
            || (coupled_streams > streams)
            || (streams < 1)
            || (coupled_streams < 0)
            || (streams > 255 - coupled_streams)
        {
            return OPUS_BAD_ARG;
        }
        if mapping.len() < channels as usize {
            return OPUS_BAD_ARG;
        }

        self.layout.nb_channels = channels;
        self.layout.nb_streams = streams;
        self.layout.nb_coupled_streams = coupled_streams;
        for i in 0..channels as usize {
            self.layout.mapping[i] = mapping[i];
        }
        if validate_layout(&self.layout) == 0 {
            return OPUS_BAD_ARG;
        }

        self.dec.clear();
        let mut i = 0;
        while i < coupled_streams {
            let mut dec = OpusDecoder::default();
            let ret = dec.init(Fs, 2);
            if ret != OPUS_OK {
                return ret;
            }
            self.dec.push(dec);
            i += 1;
        }
        while i < streams {
            let mut dec = OpusDecoder::default();
            let ret = dec.init(Fs, 1);
            if ret != OPUS_OK {
                return ret;
            }
            self.dec.push(dec);
            i += 1;
        }
        OPUS_OK
    }

    /// `opus_multistream_decoder_create()` (`src/opus_multistream_decoder.c:111`).
    ///
    /// Rust's `Box` cannot fail, so the C `OPUS_ALLOC_FAIL` return is
    /// unreachable here.
    pub fn create(
        Fs: opus_int32,
        channels: i32,
        streams: i32,
        coupled_streams: i32,
        mapping: &[u8],
        error: &mut i32,
    ) -> Option<Box<OpusMSDecoder>> {
        if (channels > 255)
            || (channels < 1)
            || (coupled_streams > streams)
            || (streams < 1)
            || (coupled_streams < 0)
            || (streams > 255 - coupled_streams)
        {
            *error = OPUS_BAD_ARG;
            return None;
        }
        let mut st = Box::new(OpusMSDecoder::default());
        let ret = st.init(Fs, channels, streams, coupled_streams, mapping);
        *error = ret;
        if ret != OPUS_OK {
            return None;
        }
        Some(st)
    }

    /// `opus_multistream_decoder_destroy()` (`src/opus_multistream_decoder.c:563`):
    /// the owned `Box` makes this a no-op, kept for contract parity.
    pub fn destroy(self) {}
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_packet_validate
 * ------------------------------------------------------------------------- */

/// `opus_multistream_packet_validate()` (`src/opus_multistream_decoder.c:144`).
///
/// Walks the self-delimited stream chain of a multistream packet and returns
/// the number of samples per stream (or a negative `OPUS_*` error).
fn opus_multistream_packet_validate(
    data: &[u8],
    len: opus_int32,
    nb_streams: i32,
    Fs: opus_int32,
) -> i32 {
    let mut samples = 0;
    let mut off = 0usize;
    let mut rem = len;

    let mut s = 0;
    while s < nb_streams {
        if rem <= 0 {
            return OPUS_INVALID_PACKET;
        }
        let parsed = match opus_packet_parse_impl(&data[off..], rem, s != nb_streams - 1) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let tmp_samples =
            opus_packet_get_nb_samples(&data[off..], parsed.packet_offset, Fs);
        if s != 0 && samples != tmp_samples {
            return OPUS_INVALID_PACKET;
        }
        samples = tmp_samples;
        off += parsed.packet_offset as usize;
        rem -= parsed.packet_offset;
        s += 1;
    }
    samples
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_decode_native
 * ------------------------------------------------------------------------- */

/// `opus_multistream_decode_native()` (`src/opus_multistream_decoder.c:180`).
///
/// `data == None` is the C `data == NULL` case; `len == 0` (or `data == NULL`)
/// selects the PLC path, exactly like C.
pub fn opus_multistream_decode_native(
    st: &mut OpusMSDecoder,
    data: Option<&[u8]>,
    len: opus_int32,
    pcm: &mut MsDecoderPcm,
    frame_size: i32,
    decode_fec: i32,
    soft_clip: i32,
) -> i32 {
    const EMPTY: &[u8] = &[];
    let data = data.unwrap_or(EMPTY);
    let mut frame_size = frame_size;

    if frame_size <= 0 {
        return OPUS_BAD_ARG;
    }
    /* Limit frame_size to avoid excessive stack allocations. */
    let mut Fs = 0i32;
    st.ctl(OPUS_GET_SAMPLE_RATE_REQUEST, &mut Fs);
    frame_size = frame_size.min(Fs / 25 * 3);

    let mut buf = vec![0.0 as opus_res; (2 * frame_size) as usize];

    let mut do_plc = 0;
    if len == 0 {
        do_plc = 1;
    }
    if len < 0 {
        return OPUS_BAD_ARG;
    }
    if do_plc == 0 && len < 2 * st.layout.nb_streams - 1 {
        return OPUS_INVALID_PACKET;
    }
    if do_plc == 0 {
        let ret = opus_multistream_packet_validate(data, len, st.layout.nb_streams, Fs);
        if ret < 0 {
            return ret;
        } else if ret > frame_size {
            return OPUS_BUFFER_TOO_SMALL;
        }
    }

    let mut off = 0usize;
    let mut rem = len;
    let mut s = 0;
    while s < st.layout.nb_streams {
        if do_plc == 0 && rem <= 0 {
            return OPUS_INTERNAL_ERROR;
        }
        let mut packet_offset = 0i32;
        let ret = opus_decode_native(
            &mut st.dec[s as usize],
            Some(&data[off..]),
            rem,
            &mut buf,
            frame_size,
            decode_fec,
            (s != st.layout.nb_streams - 1) as i32,
            Some(&mut packet_offset),
            soft_clip,
            None,
            0,
        );
        if do_plc == 0 {
            off += packet_offset as usize;
            rem -= packet_offset;
        }
        if ret <= 0 {
            return ret;
        }
        frame_size = ret;
        if s < st.layout.nb_coupled_streams {
            let mut prev = -1;
            loop {
                let chan = get_left_channel(&st.layout, s, prev);
                if chan == -1 {
                    break;
                }
                pcm.write(st.layout.nb_channels, chan, Some(&buf), 2, frame_size);
                prev = chan;
            }
            let mut prev = -1;
            loop {
                let chan = get_right_channel(&st.layout, s, prev);
                if chan == -1 {
                    break;
                }
                pcm.write(st.layout.nb_channels, chan, Some(&buf[1..]), 2, frame_size);
                prev = chan;
            }
        } else {
            let mut prev = -1;
            loop {
                let chan = get_mono_channel(&st.layout, s, prev);
                if chan == -1 {
                    break;
                }
                pcm.write(st.layout.nb_channels, chan, Some(&buf), 1, frame_size);
                prev = chan;
            }
        }
        s += 1;
    }
    /* Handle muted channels */
    let mut c = 0;
    while c < st.layout.nb_channels {
        if st.layout.mapping[c as usize] == 255 {
            pcm.write(st.layout.nb_channels, c, None, 0, frame_size);
        }
        c += 1;
    }
    frame_size
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_decode / decode24 / decode_float
 * ------------------------------------------------------------------------- */

/// `#ifdef FIXED_POINT #define OPTIONAL_CLIP 0 #else 1` — this is the float
/// build, so `opus_multistream_decode()` passes `soft_clip == 1`.
const OPTIONAL_CLIP: i32 = 1;

impl OpusMSDecoder {
    /// `opus_multistream_decode()` (`src/opus_multistream_decoder.c:354`).
    pub fn decode(
        &mut self,
        data: Option<&[u8]>,
        len: opus_int32,
        pcm: &mut [i16],
        frame_size: i32,
        decode_fec: i32,
    ) -> i32 {
        let mut out = MsDecoderPcm::Short(pcm);
        opus_multistream_decode_native(
            self,
            data,
            len,
            &mut out,
            frame_size,
            decode_fec,
            OPTIONAL_CLIP,
        )
    }

    /// `opus_multistream_decode24()` (`src/opus_multistream_decoder.c:366`).
    pub fn decode24(
        &mut self,
        data: Option<&[u8]>,
        len: opus_int32,
        pcm: &mut [i32],
        frame_size: i32,
        decode_fec: i32,
    ) -> i32 {
        let mut out = MsDecoderPcm::Int24(pcm);
        opus_multistream_decode_native(self, data, len, &mut out, frame_size, decode_fec, 0)
    }

    /// `opus_multistream_decode_float()` (`src/opus_multistream_decoder.c:378`).
    pub fn decode_float(
        &mut self,
        data: Option<&[u8]>,
        len: opus_int32,
        pcm: &mut [f32],
        frame_size: i32,
        decode_fec: i32,
    ) -> i32 {
        let mut out = MsDecoderPcm::Float(pcm);
        opus_multistream_decode_native(self, data, len, &mut out, frame_size, decode_fec, 0)
    }

    /// `OPUS_MULTISTREAM_GET_DECODER_STATE_REQUEST` — the typed equivalent of
    /// the C `opus_multistream_decoder_ctl(st, OPUS_MULTISTREAM_GET_DECODER_STATE(stream_id), &dec)`.
    ///
    /// Returns `Err(OPUS_BAD_ARG)` when `stream_id` is out of range, exactly
    /// like C (which `goto`s `bad_arg`).
    pub fn get_decoder_state(&mut self, stream_id: i32) -> Result<&mut OpusDecoder, i32> {
        if stream_id < 0 || stream_id >= self.layout.nb_streams {
            return Err(OPUS_BAD_ARG);
        }
        Ok(&mut self.dec[stream_id as usize])
    }
}

/* ------------------------------------------------------------------------- *
 * opus_multistream_decoder_ctl
 * ------------------------------------------------------------------------- */

impl OpusMSDecoder {
    /// `opus_multistream_decoder_ctl_va_list()` +
    /// `opus_multistream_decoder_ctl()` (`src/opus_multistream_decoder.c:388`).
    ///
    /// `value` is an in/out integer (`docs/internal-api.md`).  Requests whose C
    /// argument is a pointer (`OPUS_MULTISTREAM_GET_DECODER_STATE`) go through
    /// [`OpusMSDecoder::get_decoder_state`] and report `OPUS_UNIMPLEMENTED`
    /// here.
    pub fn ctl(&mut self, request: i32, value: &mut i32) -> i32 {
        let mut ret = OPUS_OK;
        match request {
            OPUS_GET_BANDWIDTH_REQUEST
            | OPUS_GET_SAMPLE_RATE_REQUEST
            | OPUS_GET_GAIN_REQUEST
            | OPUS_GET_LAST_PACKET_DURATION_REQUEST
            | OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST
            | OPUS_GET_COMPLEXITY_REQUEST => {
                /* For int32* GET params, just query the first stream */
                ret = self.dec[0].ctl(request, value);
            }
            OPUS_GET_FINAL_RANGE_REQUEST => {
                let mut acc: opus_uint32 = 0;
                let mut s = 0;
                while s < self.layout.nb_streams {
                    let mut tmp = 0i32;
                    ret = self.dec[s as usize].ctl(request, &mut tmp);
                    if ret != OPUS_OK {
                        break;
                    }
                    acc ^= tmp as opus_uint32;
                    s += 1;
                }
                *value = acc as i32;
            }
            OPUS_RESET_STATE => {
                let mut s = 0;
                while s < self.layout.nb_streams {
                    let mut dummy = 0i32;
                    ret = self.dec[s as usize].ctl(OPUS_RESET_STATE, &mut dummy);
                    if ret != OPUS_OK {
                        break;
                    }
                    s += 1;
                }
            }
            OPUS_SET_GAIN_REQUEST | OPUS_SET_COMPLEXITY_REQUEST
            | OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST => {
                /* This works for int32 params */
                let v = *value;
                let mut s = 0;
                while s < self.layout.nb_streams {
                    let mut tmp = v;
                    ret = self.dec[s as usize].ctl(request, &mut tmp);
                    if ret != OPUS_OK {
                        break;
                    }
                    s += 1;
                }
            }
            _ => {
                ret = OPUS_UNIMPLEMENTED;
            }
        }
        ret
    }
}

/* ------------------------------------------------------------------------- *
 * C-named entry points
 *
 * The `OpusMSDecoder` methods above are the implementation; these free
 * functions are the literal `src/opus_multistream_decoder.c` names, so the
 * C-to-Rust mapping stays one-to-one.
 * ------------------------------------------------------------------------- */

/// `opus_multistream_decoder_get_size()`.
pub fn opus_multistream_decoder_get_size(nb_streams: i32, nb_coupled_streams: i32) -> i32 {
    OpusMSDecoder::get_size(nb_streams, nb_coupled_streams)
}

/// `opus_multistream_decoder_init()`.
pub fn opus_multistream_decoder_init(
    st: &mut OpusMSDecoder,
    Fs: opus_int32,
    channels: i32,
    streams: i32,
    coupled_streams: i32,
    mapping: &[u8],
) -> i32 {
    st.init(Fs, channels, streams, coupled_streams, mapping)
}

/// `opus_multistream_decoder_create()`.
pub fn opus_multistream_decoder_create(
    Fs: opus_int32,
    channels: i32,
    streams: i32,
    coupled_streams: i32,
    mapping: &[u8],
    error: &mut i32,
) -> Option<Box<OpusMSDecoder>> {
    OpusMSDecoder::create(Fs, channels, streams, coupled_streams, mapping, error)
}

/// `opus_multistream_decode()`.
pub fn opus_multistream_decode(
    st: &mut OpusMSDecoder,
    data: Option<&[u8]>,
    len: opus_int32,
    pcm: &mut [i16],
    frame_size: i32,
    decode_fec: i32,
) -> i32 {
    st.decode(data, len, pcm, frame_size, decode_fec)
}

/// `opus_multistream_decode24()`.
pub fn opus_multistream_decode24(
    st: &mut OpusMSDecoder,
    data: Option<&[u8]>,
    len: opus_int32,
    pcm: &mut [i32],
    frame_size: i32,
    decode_fec: i32,
) -> i32 {
    st.decode24(data, len, pcm, frame_size, decode_fec)
}

/// `opus_multistream_decode_float()`.
pub fn opus_multistream_decode_float(
    st: &mut OpusMSDecoder,
    data: Option<&[u8]>,
    len: opus_int32,
    pcm: &mut [f32],
    frame_size: i32,
    decode_fec: i32,
) -> i32 {
    st.decode_float(data, len, pcm, frame_size, decode_fec)
}

/// `opus_multistream_decoder_ctl()` (the `_va_list` body plus its wrapper).
pub fn opus_multistream_decoder_ctl(
    st: &mut OpusMSDecoder,
    request: i32,
    value: &mut i32,
) -> i32 {
    st.ctl(request, value)
}

/// `opus_multistream_decoder_destroy()`: no-op with an owned `Box`.
pub fn opus_multistream_decoder_destroy(_st: Box<OpusMSDecoder>) {}

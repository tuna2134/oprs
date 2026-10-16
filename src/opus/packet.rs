//! Mirror of `src/opus.c` (packet framing), together with
//!
//! * the packet query helpers that live in `src/opus_decoder.c`
//!   (`opus_packet_get_bandwidth`, `…_nb_channels`, `…_nb_frames`,
//!   `…_nb_samples`, `…_has_lbrr` and the static `opus_packet_get_mode`),
//! * the extension helpers from `src/extensions.c` that `src/repacketizer.c`
//!   calls unconditionally (`opus_packet_extensions_count/parse/generate` and
//!   the `OpusExtensionIterator` they are built on).
//!
//! `opus_pcm_soft_clip()` is ported too (it lives in `src/opus.c`).
//!
//! ---
//!
//! Copyright (c) 2011 Xiph.Org Foundation, Skype Limited
//! Copyright (c) 2024 Arm Limited
//! Written by Jean-Marc Valin and Koen Vos
//!
//! Copyright (c) 2022 Amazon.com, Inc. or its affiliates.
//! Written by Jean-Marc Valin   (`src/extensions.c`)
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions
//! are met:
//!
//! - Redistributions of source code must retain the above copyright
//! notice, this list of conditions and the following disclaimer.
//!
//! - Redistributions in binary form must reproduce the above copyright
//! notice, this list of conditions and the following disclaimer in the
//! documentation and/or other materials provided with the distribution.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
//! ``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
//! LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
//! A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
//! OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
//! PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
//! PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
//! LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//!
//! This file is a transliteration of the corresponding C sources into safe
//! Rust; every function keeps its C name and the arithmetic order of the
//! original.

use crate::celt::arch::*;
use crate::types::*;

/* Error codes, from include/opus_defines.h. */
pub const OPUS_OK: i32 = 0;
pub const OPUS_BAD_ARG: i32 = -1;
pub const OPUS_BUFFER_TOO_SMALL: i32 = -2;
pub const OPUS_INTERNAL_ERROR: i32 = -3;
pub const OPUS_INVALID_PACKET: i32 = -4;
pub const OPUS_UNIMPLEMENTED: i32 = -5;

/* Bandwidth values, from include/opus_defines.h. */
pub const OPUS_BANDWIDTH_NARROWBAND: i32 = 1101;
pub const OPUS_BANDWIDTH_MEDIUMBAND: i32 = 1102;
pub const OPUS_BANDWIDTH_WIDEBAND: i32 = 1103;
pub const OPUS_BANDWIDTH_SUPERWIDEBAND: i32 = 1104;
pub const OPUS_BANDWIDTH_FULLBAND: i32 = 1105;

/* Packet modes, from src/opus_private.h. */
pub const MODE_SILK_ONLY: i32 = 1000;
pub const MODE_HYBRID: i32 = 1001;
pub const MODE_CELT_ONLY: i32 = 1002;

/// `encode_size()` from `src/opus.c`.
pub fn encode_size(size: i32, data: &mut [u8]) -> i32 {
    if size < 252 {
        data[0] = size as u8;
        1
    } else {
        data[0] = (252 + (size & 0x3)) as u8;
        let b0 = data[0] as i32;
        data[1] = ((size - b0) >> 2) as u8;
        2
    }
}

/// `parse_size()` from `src/opus.c`.
pub fn parse_size(data: &[u8], len: opus_int32, size: &mut opus_int16) -> opus_int32 {
    if len < 1 {
        *size = -1;
        -1
    } else if data[0] < 252 {
        *size = data[0] as opus_int16;
        1
    } else if len < 2 {
        *size = -1;
        -1
    } else {
        *size = 4 * data[1] as opus_int16 + data[0] as opus_int16;
        2
    }
}

/// `opus_packet_get_samples_per_frame()` from `src/opus.c`.
///
/// Returns `OPUS_BAD_ARG` for an empty slice (the C reads `data[0]`
/// unconditionally; an empty packet is not a valid input).
pub fn opus_packet_get_samples_per_frame(data: &[u8], Fs: opus_int32) -> opus_int32 {
    if data.is_empty() {
        return OPUS_BAD_ARG;
    }
    let audiosize;
    if data[0] & 0x80 != 0 {
        let audiosize_idx = ((data[0] >> 3) & 0x3) as u32;
        audiosize = Fs.wrapping_shl(audiosize_idx) / 400;
    } else if (data[0] & 0x60) == 0x60 {
        audiosize = if data[0] & 0x08 != 0 { Fs / 50 } else { Fs / 100 };
    } else {
        let audiosize_idx = ((data[0] >> 3) & 0x3) as i32;
        if audiosize_idx == 3 {
            audiosize = Fs.wrapping_mul(60) / 1000;
        } else {
            audiosize = Fs.wrapping_shl(audiosize_idx as u32) / 100;
        }
    }
    audiosize
}

/// `opus_packet_get_bandwidth()` from `src/opus_decoder.c`.
///
/// Returns `OPUS_BAD_ARG` for an empty slice.
pub fn opus_packet_get_bandwidth(data: &[u8]) -> i32 {
    if data.is_empty() {
        return OPUS_BAD_ARG;
    }
    let bandwidth;
    if data[0] & 0x80 != 0 {
        bandwidth = OPUS_BANDWIDTH_MEDIUMBAND + ((data[0] >> 5) & 0x3) as i32;
        if bandwidth == OPUS_BANDWIDTH_MEDIUMBAND {
            return OPUS_BANDWIDTH_NARROWBAND;
        }
    } else if (data[0] & 0x60) == 0x60 {
        bandwidth = if data[0] & 0x10 != 0 {
            OPUS_BANDWIDTH_FULLBAND
        } else {
            OPUS_BANDWIDTH_SUPERWIDEBAND
        };
    } else {
        bandwidth = OPUS_BANDWIDTH_NARROWBAND + ((data[0] >> 5) & 0x3) as i32;
    }
    bandwidth
}

/// `opus_packet_get_nb_channels()` from `src/opus_decoder.c`.
///
/// Returns `OPUS_BAD_ARG` for an empty slice.
pub fn opus_packet_get_nb_channels(data: &[u8]) -> i32 {
    if data.is_empty() {
        return OPUS_BAD_ARG;
    }
    if data[0] & 0x4 != 0 { 2 } else { 1 }
}

/// `opus_packet_get_nb_frames()` from `src/opus_decoder.c`.
pub fn opus_packet_get_nb_frames(packet: &[u8], len: opus_int32) -> i32 {
    if len < 1 || packet.is_empty() {
        return OPUS_BAD_ARG;
    }
    let count = (packet[0] & 0x3) as i32;
    if count == 0 {
        1
    } else if count != 3 {
        2
    } else if len < 2 || packet.len() < 2 {
        OPUS_INVALID_PACKET
    } else {
        (packet[1] & 0x3F) as i32
    }
}

/// `opus_packet_get_nb_samples()` from `src/opus_decoder.c`.
pub fn opus_packet_get_nb_samples(packet: &[u8], len: opus_int32, Fs: opus_int32) -> i32 {
    let count = opus_packet_get_nb_frames(packet, len);
    if count < 0 {
        return count;
    }
    let samples = count.wrapping_mul(opus_packet_get_samples_per_frame(packet, Fs));
    /* Can't have more than 120 ms */
    if samples.wrapping_mul(25) > Fs.wrapping_mul(3) {
        OPUS_INVALID_PACKET
    } else {
        samples
    }
}

/// `opus_packet_get_mode()` (static in `src/opus_decoder.c`).
pub fn opus_packet_get_mode(data: &[u8]) -> i32 {
    if data.is_empty() {
        return MODE_SILK_ONLY;
    }
    if data[0] & 0x80 != 0 {
        MODE_CELT_ONLY
    } else if (data[0] & 0x60) == 0x60 {
        MODE_HYBRID
    } else {
        MODE_SILK_ONLY
    }
}

/// `opus_packet_has_lbrr()` from `src/opus_decoder.c`.
pub fn opus_packet_has_lbrr(packet: &[u8], len: opus_int32) -> i32 {
    let mut frames: [&[u8]; 48] = [&[]; 48];
    let mut size = [0i16; 48];
    let mut nb_frames = 1;

    if packet.is_empty() {
        return OPUS_BAD_ARG;
    }
    let packet_mode = opus_packet_get_mode(packet);
    if packet_mode == MODE_CELT_ONLY {
        return 0;
    }
    let packet_frame_size = opus_packet_get_samples_per_frame(packet, 48000);
    if packet_frame_size > 960 {
        nb_frames = packet_frame_size / 960;
    }
    let packet_stream_channels = opus_packet_get_nb_channels(packet);
    let mut tmp_toc = 0u8;
    let mut tmp_offset = 0i32;
    let ret = opus_packet_parse(packet, len, &mut tmp_toc, &mut frames, &mut size, &mut tmp_offset);
    if ret <= 0 {
        return ret;
    }
    if size[0] == 0 {
        return 0;
    }
    let mut lbrr = ((frames[0][0] as i32) >> (7 - nb_frames)) & 0x1;
    if packet_stream_channels == 2 {
        /* The C shifts by a possibly negative amount here (e.g. -6 for a
           120 ms stereo packet); x86 masks the count modulo 32, which is what
           `wrapping_shr` reproduces. */
        lbrr |= ((frames[0][0] as i32).wrapping_shr((6 - 2 * nb_frames) as u32)) & 0x1;
    }
    lbrr
}

/// The result of `opus_packet_parse_impl()`.
///
/// In C the function fills a caller-provided `frames[]`/`size[]` and returns
/// write-out parameters through pointers; here the frames are returned as
/// subslices of the packet and the offsets are returned directly.
pub struct ParsedPacket<'a> {
    /// TOC byte.
    pub toc: u8,
    /// Number of frames (`ret` of the C function).
    pub count: opus_int32,
    /// The `count` frame payloads.
    pub frames: [&'a [u8]; 48],
    /// The `count` frame lengths (`opus_int16 size[48]`).
    pub size: [opus_int16; 48],
    /// `payload_offset` output.
    pub payload_offset: opus_int32,
    /// `packet_offset` output (`pad + (data-data0)`, used by multistream).
    pub packet_offset: opus_int32,
    /// The padding region, i.e. `*padding[0..*padding_len]`.
    pub padding: &'a [u8],
}

/// `opus_packet_parse_impl()` from `src/opus.c`.
pub fn opus_packet_parse_impl<'a>(
    packet: &'a [u8],
    len: opus_int32,
    self_delimited: bool,
) -> Result<ParsedPacket<'a>, opus_int32> {
    let mut size = [0i16; 48];
    let mut i: opus_int32;
    let count: opus_int32;
    let mut cbr = false;
    let mut last_size: opus_int32;
    let mut pad: opus_int32 = 0;

    if len < 0 {
        return Err(OPUS_BAD_ARG);
    }
    /* C would read past the caller's buffer if `len` exceeded it (undefined
       behaviour); clamp so that the port can never index out of bounds. */
    let len = len.min(packet.len() as opus_int32);
    let data0 = &packet[..len as usize];
    if len == 0 {
        return Err(OPUS_INVALID_PACKET);
    }

    let framesize = opus_packet_get_samples_per_frame(data0, 48000);

    let mut data = data0;
    let mut len = len;
    let toc = data[0];
    data = &data[1..];
    len -= 1;
    last_size = len;
    match toc & 0x3 {
        /* One frame */
        0 => {
            count = 1;
        }
        /* Two CBR frames */
        1 => {
            count = 2;
            cbr = true;
            if !self_delimited {
                if len & 0x1 != 0 {
                    return Err(OPUS_INVALID_PACKET);
                }
                last_size = len / 2;
                /* If last_size doesn't fit in size[0], we'll catch it later */
                size[0] = last_size as opus_int16;
            }
        }
        /* Two VBR frames */
        2 => {
            count = 2;
            let bytes = parse_size(data, len, &mut size[0]);
            len -= bytes;
            if size[0] < 0 || size[0] as opus_int32 > len {
                return Err(OPUS_INVALID_PACKET);
            }
            data = &data[bytes as usize..];
            last_size = len - size[0] as opus_int32;
        }
        /* Multiple CBR/VBR frames (from 0 to 120 ms) */
        _ => {
            /* case 3 */
            if len < 1 {
                return Err(OPUS_INVALID_PACKET);
            }
            /* Number of frames encoded in bits 0 to 5 */
            let ch = data[0];
            data = &data[1..];
            count = (ch & 0x3F) as opus_int32;
            if count <= 0 || framesize.wrapping_mul(count) > 5760 {
                return Err(OPUS_INVALID_PACKET);
            }
            len -= 1;
            /* Padding flag is bit 6 */
            if ch & 0x40 != 0 {
                let mut p;
                loop {
                    if len <= 0 {
                        return Err(OPUS_INVALID_PACKET);
                    }
                    p = data[0];
                    data = &data[1..];
                    len -= 1;
                    let tmp = if p == 255 { 254 } else { p as opus_int32 };
                    len -= tmp;
                    pad += tmp;
                    if p != 255 {
                        break;
                    }
                }
            }
            if len < 0 {
                return Err(OPUS_INVALID_PACKET);
            }
            /* VBR flag is bit 7 */
            cbr = (ch & 0x80) == 0;
            if !cbr {
                /* VBR case */
                last_size = len;
                i = 0;
                while i < count - 1 {
                    let bytes = parse_size(data, len, &mut size[i as usize]);
                    len -= bytes;
                    if size[i as usize] < 0 || size[i as usize] as opus_int32 > len {
                        return Err(OPUS_INVALID_PACKET);
                    }
                    data = &data[bytes as usize..];
                    last_size -= bytes + size[i as usize] as opus_int32;
                    i += 1;
                }
                if last_size < 0 {
                    return Err(OPUS_INVALID_PACKET);
                }
            } else if !self_delimited {
                /* CBR case */
                last_size = len / count;
                if last_size * count != len {
                    return Err(OPUS_INVALID_PACKET);
                }
                i = 0;
                while i < count - 1 {
                    size[i as usize] = last_size as opus_int16;
                    i += 1;
                }
            }
        }
    }
    /* Self-delimited framing has an extra size for the last frame. */
    if self_delimited {
        let bytes = parse_size(data, len, &mut size[(count - 1) as usize]);
        len -= bytes;
        if size[(count - 1) as usize] < 0 || size[(count - 1) as usize] as opus_int32 > len {
            return Err(OPUS_INVALID_PACKET);
        }
        data = &data[bytes as usize..];
        /* For CBR packets, apply the size to all the frames. */
        if cbr {
            if size[(count - 1) as usize] as opus_int32 * count > len {
                return Err(OPUS_INVALID_PACKET);
            }
            i = 0;
            while i < count - 1 {
                size[i as usize] = size[(count - 1) as usize];
                i += 1;
            }
        } else if bytes + size[(count - 1) as usize] as opus_int32 > last_size {
            return Err(OPUS_INVALID_PACKET);
        }
    } else {
        /* Because it's not encoded explicitly, it's possible the size of the
           last packet (or all the packets, for the CBR case) is larger than
           1275. Reject them here.*/
        if last_size > 1275 {
            return Err(OPUS_INVALID_PACKET);
        }
        size[(count - 1) as usize] = last_size as opus_int16;
    }

    let payload_offset = (data0.len() - data.len()) as opus_int32;

    let mut frames: [&'a [u8]; 48] = [&[]; 48];
    let mut cur = data;
    i = 0;
    while i < count {
        if size[i as usize] < 0 {
            return Err(OPUS_INVALID_PACKET);
        }
        let s = size[i as usize] as usize;
        if s > cur.len() {
            return Err(OPUS_INVALID_PACKET);
        }
        frames[i as usize] = &cur[..s];
        cur = &cur[s..];
        i += 1;
    }

    let padding = &cur[..(pad as usize).min(cur.len())];
    let packet_offset = pad + (data0.len() - cur.len()) as opus_int32;

    Ok(ParsedPacket {
        toc,
        count,
        frames,
        size,
        payload_offset,
        packet_offset,
        padding,
    })
}

/// `opus_packet_parse()` from `src/opus.c`.
///
/// `frames` must have room for 48 entries; the leading `count` entries are
/// filled with subslices of `packet`.  `size` and `payload_offset` receive the
/// matching values.  Returns the frame count, or a negative `OPUS_*` code.
pub fn opus_packet_parse<'a>(
    packet: &'a [u8],
    len: opus_int32,
    out_toc: &mut u8,
    frames: &mut [&'a [u8]],
    size: &mut [opus_int16],
    payload_offset: &mut opus_int32,
) -> i32 {
    match opus_packet_parse_impl(packet, len, false) {
        Ok(p) => {
            *out_toc = p.toc;
            *payload_offset = p.payload_offset;
            let mut i = 0usize;
            while i < p.count as usize {
                if i < frames.len() {
                    frames[i] = p.frames[i];
                }
                if i < size.len() {
                    size[i] = p.size[i];
                }
                i += 1;
            }
            p.count
        }
        Err(e) => e,
    }
}

/// `opus_pcm_soft_clip()` from `src/opus.c` (with `arch == 0`, so
/// `opus_limit2_checkwithin1()` is the plain C version that clamps to
/// [-2, 2] and always reports "not all within [-1, 1]").
///
/// `pcm` holds `frame_size * channels` samples; `softclip_mem` holds the
/// per-channel `a` from the previous call.  If the slices are too short for
/// `frame_size`/`channels` the call is ignored rather than panicking (the C
/// would read out of bounds).
pub fn opus_pcm_soft_clip(pcm: &mut [f32], frame_size: i32, channels: i32, softclip_mem: &mut [f32]) {
    let N = frame_size;
    let C = channels;
    if C < 1 || N < 1 {
        return;
    }
    if C as usize > softclip_mem.len() {
        return;
    }
    if (N as i64 * C as i64) as usize > pcm.len() {
        return;
    }

    /* all_within_neg1pos1 = opus_limit2_checkwithin1(_x, N*C, arch);  (arch==0) */
    let mut k: i32 = 0;
    while k < N * C {
        let mut clippedVal = pcm[k as usize];
        clippedVal = FMAX(-2.0f32, clippedVal);
        clippedVal = FMIN(2.0f32, clippedVal);
        pcm[k as usize] = clippedVal;
        k += 1;
    }
    let all_within_neg1pos1 = false;

    let mut c: i32 = 0;
    while c < C {
        let mut a: f32;
        let x0: f32;
        let mut curr: i32;

        a = softclip_mem[c as usize];
        /* Continue applying the non-linearity from the previous frame to avoid
           any discontinuity. */
        let mut i: i32 = 0;
        while i < N {
            let v = pcm[(c + i * C) as usize];
            if v * a >= 0.0 {
                break;
            }
            pcm[(c + i * C) as usize] = v + a * v * v;
            i += 1;
        }

        curr = 0;
        x0 = pcm[c as usize];
        loop {
            let mut start: i32;
            let mut end: i32;
            let mut maxval: f32;
            let mut peak_pos: i32;
            /* Detection for early exit can be skipped if hinted by
               `all_within_neg1pos1` */
            if all_within_neg1pos1 {
                i = N;
            } else {
                i = curr;
                while i < N {
                    let v = pcm[(c + i * C) as usize];
                    if v > 1.0 || v < -1.0 {
                        break;
                    }
                    i += 1;
                }
            }
            if i == N {
                a = 0.0;
                break;
            }
            peak_pos = i;
            start = i;
            end = i;
            maxval = ABS16(pcm[(c + i * C) as usize]);
            /* Look for first zero crossing before clipping */
            while start > 0
                && pcm[(c + i * C) as usize] * pcm[(c + (start - 1) * C) as usize] >= 0.0
            {
                start -= 1;
            }
            /* Look for first zero crossing after clipping */
            while end < N && pcm[(c + i * C) as usize] * pcm[(c + end * C) as usize] >= 0.0 {
                /* Look for other peaks until the next zero-crossing. */
                if ABS16(pcm[(c + end * C) as usize]) > maxval {
                    maxval = ABS16(pcm[(c + end * C) as usize]);
                    peak_pos = end;
                }
                end += 1;
            }
            /* Detect the special case where we clip before the first zero
               crossing */
            let special = if start == 0 && pcm[(c + i * C) as usize] * pcm[c as usize] >= 0.0 {
                1
            } else {
                0
            };

            /* Compute a such that maxval + a*maxval^2 = 1 */
            a = (maxval - 1.0) / (maxval * maxval);
            /* Slightly boost "a" by 2^-22. This is just enough to ensure
               -ffast-math does not cause output values larger than +/-1, but
               small enough not to matter even for 24-bit output.  */
            a += a * 2.4e-7f32;
            if pcm[(c + i * C) as usize] > 0.0 {
                a = -a;
            }
            /* Apply soft clipping */
            let mut j = start;
            while j < end {
                let v = pcm[(c + j * C) as usize];
                pcm[(c + j * C) as usize] = v + a * v * v;
                j += 1;
            }

            if special != 0 && peak_pos >= 2 {
                /* Add a linear ramp from the first sample to the signal peak.
                   This avoids a discontinuity at the beginning of the frame. */
                let mut offset = x0 - pcm[c as usize];
                let delta = offset / peak_pos as f32;
                let mut j = curr;
                while j < peak_pos {
                    offset -= delta;
                    pcm[(c + j * C) as usize] += offset;
                    pcm[(c + j * C) as usize] =
                        MAX16(-1.0f32, MIN16(1.0f32, pcm[(c + j * C) as usize]));
                    j += 1;
                }
            }
            curr = end;
            if curr == N {
                break;
            }
        }
        softclip_mem[c as usize] = a;
        c += 1;
    }
}

// ---------------------------------------------------------------------------
// Extensions (`src/extensions.c`).
//
// `opus_extension_iterator_reset()`, `opus_extension_iterator_set_frame_max()`,
// `opus_extension_iterator_find()`, `opus_packet_extensions_count_ext()` and
// `opus_packet_extensions_parse_ext()` are not used by `src/repacketizer.c` in
// this configuration and are therefore not ported.
// ---------------------------------------------------------------------------

/// `opus_extension_data` from `src/opus_private.h`.
///
/// The C struct points into the packet; here the payload is owned.
#[derive(Clone, Default)]
pub struct OpusExtensionData {
    pub id: i32,
    pub frame: i32,
    pub data: Vec<u8>,
    pub len: i32,
}

/// `OpusExtensionIterator` from `src/opus_private.h`.
///
/// The C pointers into the extension buffer become indices into `data`.
pub struct OpusExtensionIterator<'a> {
    pub data: &'a [u8],
    pub curr_data: usize,
    pub repeat_data: usize,
    pub last_long: Option<usize>,
    pub src_data: usize,
    pub len: i32,
    pub curr_len: i32,
    pub repeat_len: i32,
    pub src_len: i32,
    pub trailing_short_len: i32,
    pub nb_frames: i32,
    pub frame_max: i32,
    pub curr_frame: i32,
    pub repeat_frame: i32,
    pub repeat_l: u8,
}

impl<'a> OpusExtensionIterator<'a> {
    /// `opus_extension_iterator_init()`.
    pub fn new(data: &'a [u8], len: i32, nb_frames: i32) -> Self {
        /* C asserts len >= 0 and that the buffer holds `len` bytes; clamp so
           that inconsistent callers cannot index out of bounds. */
        let len = len.min(data.len() as i32);
        OpusExtensionIterator {
            data,
            repeat_data: 0,
            curr_data: 0,
            last_long: None,
            src_data: 0,
            curr_len: len,
            len,
            repeat_len: 0,
            src_len: 0,
            trailing_short_len: 0,
            frame_max: nb_frames,
            nb_frames,
            repeat_frame: 0,
            curr_frame: 0,
            repeat_l: 0,
        }
    }
}

/// `skip_extension_payload()` from `src/extensions.c`.
fn skip_extension_payload(
    data: &[u8],
    pos: &mut usize,
    len: i32,
    pheader_size: &mut i32,
    id_byte: i32,
    trailing_short_len: i32,
) -> i32 {
    let mut len = len;
    let mut header_size = 0i32;
    let id = id_byte >> 1;
    let L = id_byte & 1;
    if (id == 0 && L == 1) || id == 2 {
        /* Nothing to do. */
    } else if id > 0 && id < 32 {
        if len < L {
            return -1;
        }
        *pos += L as usize;
        len -= L;
    } else if L == 0 {
        if len < trailing_short_len {
            return -1;
        }
        *pos += (len - trailing_short_len) as usize;
        len = trailing_short_len;
    } else {
        let mut bytes = 0i32;
        let mut lacing;
        loop {
            if len < 1 {
                return -1;
            }
            lacing = data[*pos] as i32;
            *pos += 1;
            bytes += lacing;
            header_size += 1;
            len -= lacing + 1;
            if lacing != 255 {
                break;
            }
        }
        if len < 0 {
            return -1;
        }
        *pos += bytes as usize;
    }
    *pheader_size = header_size;
    len
}

/// `skip_extension()` from `src/extensions.c`.
fn skip_extension(data: &[u8], pos: &mut usize, len: i32, pheader_size: &mut i32) -> i32 {
    if len == 0 {
        *pheader_size = 0;
        return 0;
    }
    if len < 1 {
        return -1;
    }
    let id_byte = data[*pos] as i32;
    *pos += 1;
    let len = len - 1;
    let len = skip_extension_payload(data, pos, len, pheader_size, id_byte, 0);
    if len >= 0 {
        *pheader_size += 1;
    }
    len
}

/// `opus_extension_iterator_next_repeat()` from `src/extensions.c`.
fn opus_extension_iterator_next_repeat(
    iter: &mut OpusExtensionIterator,
    mut ext: Option<&mut OpusExtensionData>,
) -> i32 {
    let mut header_size = 0i32;
    debug_assert!(iter.repeat_frame > 0);
    while iter.repeat_frame < iter.nb_frames {
        while iter.src_len > 0 {
            let curr_data0;
            let repeat_id_byte;
            repeat_id_byte = iter.data[iter.src_data] as i32;
            iter.src_len =
                skip_extension(iter.data, &mut iter.src_data, iter.src_len, &mut header_size);
            /* We skipped this extension earlier, so it should not fail now. */
            debug_assert!(iter.src_len >= 0);
            /* Don't repeat padding or frame separators with a 0 increment. */
            if repeat_id_byte <= 3 {
                continue;
            }
            let mut rid = repeat_id_byte;
            /* If the "Repeat These Extensions" extension had L == 0 and this
               is the last repeated long extension, then force decoding the
               payload with L = 0. */
            if iter.repeat_l == 0
                && iter.repeat_frame + 1 >= iter.nb_frames
                && Some(iter.src_data) == iter.last_long
            {
                rid &= !1;
            }
            curr_data0 = iter.curr_data;
            iter.curr_len = skip_extension_payload(
                iter.data,
                &mut iter.curr_data,
                iter.curr_len,
                &mut header_size,
                rid,
                iter.trailing_short_len,
            );
            if iter.curr_len < 0 {
                return OPUS_INVALID_PACKET;
            }
            /* If we were asked to stop at frame_max, skip extensions for later
               frames. */
            if iter.repeat_frame >= iter.frame_max {
                continue;
            }
            if let Some(e) = ext.as_deref_mut() {
                e.id = rid >> 1;
                e.frame = iter.repeat_frame;
                let start = curr_data0 + header_size as usize;
                let n = iter.curr_data - curr_data0 - header_size as usize;
                e.data = iter.data[start..start + n].to_vec();
                e.len = n as i32;
            }
            return 1;
        }
        /* We finished repeating the extensions for this frame. */
        iter.src_data = iter.repeat_data;
        iter.src_len = iter.repeat_len;
        iter.repeat_frame += 1;
    }
    /* We finished repeating extensions. */
    iter.repeat_data = iter.curr_data;
    iter.last_long = None;
    /* If L == 0, advance the frame number to handle the case where we did
       not consume all of the data with an L == 0 long extension. */
    if iter.repeat_l == 0 {
        iter.curr_frame += 1;
        /* Ignore additional padding if this was already the last frame. */
        if iter.curr_frame >= iter.nb_frames {
            iter.curr_len = 0;
        }
    }
    iter.repeat_frame = 0;
    0
}

/// `opus_extension_iterator_next()` from `src/extensions.c`.
pub fn opus_extension_iterator_next(
    iter: &mut OpusExtensionIterator,
    mut ext: Option<&mut OpusExtensionData>,
) -> i32 {
    let mut header_size = 0i32;
    if iter.curr_len < 0 {
        return OPUS_INVALID_PACKET;
    }
    if iter.repeat_frame > 0 {
        /* We are in the process of repeating some extensions. */
        let ret = opus_extension_iterator_next_repeat(iter, ext.as_deref_mut());
        if ret != 0 {
            return ret;
        }
    }
    /* Checking this here allows opus_extension_iterator_set_frame_max() to be
       called at any point. */
    if iter.curr_frame >= iter.frame_max {
        return 0;
    }
    while iter.curr_len > 0 {
        let curr_data0;
        let id;
        let L;
        curr_data0 = iter.curr_data;
        id = (iter.data[curr_data0] >> 1) as i32;
        L = (iter.data[curr_data0] & 1) as i32;
        iter.curr_len =
            skip_extension(iter.data, &mut iter.curr_data, iter.curr_len, &mut header_size);
        if iter.curr_len < 0 {
            return OPUS_INVALID_PACKET;
        }
        debug_assert!(iter.curr_data as i32 == iter.len - iter.curr_len);
        if id == 1 {
            if L == 0 {
                iter.curr_frame += 1;
            } else {
                /* A frame increment of 0 is a no-op. */
                if iter.data[curr_data0 + 1] == 0 {
                    continue;
                }
                iter.curr_frame += iter.data[curr_data0 + 1] as i32;
            }
            if iter.curr_frame >= iter.nb_frames {
                iter.curr_len = -1;
                return OPUS_INVALID_PACKET;
            }
            /* If we were asked to stop at frame_max, skip extensions for later
               frames. */
            if iter.curr_frame >= iter.frame_max {
                iter.curr_len = 0;
            }
            iter.repeat_data = iter.curr_data;
            iter.last_long = None;
            iter.trailing_short_len = 0;
        } else if id == 2 {
            iter.repeat_l = L as u8;
            iter.repeat_frame = iter.curr_frame + 1;
            iter.repeat_len = (curr_data0 - iter.repeat_data) as i32;
            iter.src_data = iter.repeat_data;
            iter.src_len = iter.repeat_len;
            let ret = opus_extension_iterator_next_repeat(iter, ext.as_deref_mut());
            if ret != 0 {
                return ret;
            }
        } else if id > 2 {
            /* Update the location of the last long extension.
               This lets us know when we need to modify the last L flag if we
               repeat these extensions with L=0. */
            if id >= 32 {
                iter.last_long = Some(iter.curr_data);
                iter.trailing_short_len = 0;
            } else {
                /* Otherwise, keep track of how many payload bytes follow the
                   last long extension. */
                iter.trailing_short_len += L;
            }
            if let Some(e) = ext.as_deref_mut() {
                e.id = id;
                e.frame = iter.curr_frame;
                let start = curr_data0 + header_size as usize;
                let n = iter.curr_data - curr_data0 - header_size as usize;
                e.data = iter.data[start..start + n].to_vec();
                e.len = n as i32;
            }
            return 1;
        }
    }
    0
}

/// `opus_packet_extensions_count()` from `src/extensions.c`.
pub fn opus_packet_extensions_count(data: &[u8], len: i32, nb_frames: i32) -> i32 {
    let mut iter = OpusExtensionIterator::new(data, len, nb_frames);
    let mut count = 0i32;
    while opus_extension_iterator_next(&mut iter, None) > 0 {
        count += 1;
    }
    count
}

/// `opus_packet_extensions_parse()` from `src/extensions.c`.
///
/// `nb_extensions` is the capacity of `extensions` on entry and receives the
/// number of extensions actually found.
pub fn opus_packet_extensions_parse(
    data: &[u8],
    len: i32,
    extensions: &mut [OpusExtensionData],
    nb_extensions: &mut i32,
    nb_frames: i32,
) -> i32 {
    let mut iter = OpusExtensionIterator::new(data, len, nb_frames);
    let mut count = 0i32;
    loop {
        let mut ext = OpusExtensionData::default();
        let ret = opus_extension_iterator_next(&mut iter, Some(&mut ext));
        if ret <= 0 {
            *nb_extensions = count;
            return ret;
        }
        if count == *nb_extensions || count as usize >= extensions.len() {
            return OPUS_BUFFER_TOO_SMALL;
        }
        extensions[count as usize] = ext;
        count += 1;
    }
}

/// `write_extension_payload()` from `src/extensions.c`.
fn write_extension_payload(
    data: &mut Option<&mut [u8]>,
    len: i32,
    mut pos: i32,
    ext: &OpusExtensionData,
    last: i32,
) -> i32 {
    debug_assert!(ext.id >= 3 && ext.id <= 127);
    if ext.id < 32 {
        if ext.len < 0 || ext.len > 1 {
            return OPUS_BAD_ARG;
        }
        if ext.len > 0 {
            if len - pos < ext.len {
                return OPUS_BUFFER_TOO_SMALL;
            }
            if ext.data.is_empty() {
                return OPUS_BAD_ARG;
            }
            if let Some(d) = data.as_deref_mut() {
                d[pos as usize] = ext.data[0];
            }
            pos += 1;
        }
    } else {
        if ext.len < 0 || ext.len as usize > ext.data.len() {
            return OPUS_BAD_ARG;
        }
        let mut length_bytes = 1 + ext.len / 255;
        if last != 0 {
            length_bytes = 0;
        }
        if len - pos < length_bytes + ext.len {
            return OPUS_BUFFER_TOO_SMALL;
        }
        if last == 0 {
            let mut j = 0;
            while j < ext.len / 255 {
                if let Some(d) = data.as_deref_mut() {
                    d[pos as usize] = 255;
                }
                pos += 1;
                j += 1;
            }
            if let Some(d) = data.as_deref_mut() {
                d[pos as usize] = (ext.len % 255) as u8;
            }
            pos += 1;
        }
        if let Some(d) = data.as_deref_mut() {
            let n = ext.len as usize;
            d[pos as usize..pos as usize + n].copy_from_slice(&ext.data[..n]);
        }
        pos += ext.len;
    }
    pos
}

/// `write_extension()` from `src/extensions.c`.
fn write_extension(
    data: &mut Option<&mut [u8]>,
    len: i32,
    pos: i32,
    ext: &OpusExtensionData,
    last: i32,
) -> i32 {
    if len - pos < 1 {
        return OPUS_BUFFER_TOO_SMALL;
    }
    debug_assert!(ext.id >= 3 && ext.id <= 127);
    if let Some(d) = data.as_deref_mut() {
        d[pos as usize] =
            ((ext.id << 1) + if ext.id < 32 { ext.len } else { if last == 0 { 1 } else { 0 } })
                as u8;
    }
    write_extension_payload(data, len, pos + 1, ext, last)
}

/// `opus_packet_extensions_generate()` from `src/extensions.c`.
///
/// `data == None` computes the required length without writing anything, as
/// the C does with a NULL pointer.
pub fn opus_packet_extensions_generate(
    mut data: Option<&mut [u8]>,
    len: i32,
    extensions: &[OpusExtensionData],
    nb_extensions: i32,
    nb_frames: i32,
    pad: i32,
) -> i32 {
    let mut frame_min_idx = [0i32; 48];
    let mut frame_max_idx = [0i32; 48];
    let mut frame_repeat_idx = [0i32; 48];
    let mut curr_frame = 0i32;
    let mut pos = 0i32;
    let mut written = 0i32;
    let mut f: i32;
    let mut i: i32;

    debug_assert!(len >= 0);
    if nb_frames > 48 {
        return OPUS_BAD_ARG;
    }

    /* Do a little work up-front to make this O(nb_extensions) instead of
       O(nb_extensions*nb_frames) so long as the extensions are in frame
       order (without requiring that they be in frame order). */
    f = 0;
    while f < nb_frames {
        frame_min_idx[f as usize] = nb_extensions;
        frame_max_idx[f as usize] = 0;
        f += 1;
    }
    i = 0;
    while i < nb_extensions {
        f = extensions[i as usize].frame;
        if f < 0 || f >= nb_frames {
            return OPUS_BAD_ARG;
        }
        if extensions[i as usize].id < 3 || extensions[i as usize].id > 127 {
            return OPUS_BAD_ARG;
        }
        frame_min_idx[f as usize] = IMIN(frame_min_idx[f as usize], i);
        frame_max_idx[f as usize] = IMAX(frame_max_idx[f as usize], i + 1);
        i += 1;
    }
    f = 0;
    while f < nb_frames {
        frame_repeat_idx[f as usize] = frame_min_idx[f as usize];
        f += 1;
    }
    f = 0;
    while f < nb_frames {
        let mut last_long_idx: i32;
        let mut repeat_count: i32;
        repeat_count = 0;
        last_long_idx = -1;
        if f + 1 < nb_frames {
            i = frame_min_idx[f as usize];
            while i < frame_max_idx[f as usize] {
                if extensions[i as usize].frame == f {
                    let g;
                    /* Test if we can repeat this extension in future frames. */
                    let mut gg = f + 1;
                    while gg < nb_frames {
                        if frame_repeat_idx[gg as usize] >= frame_max_idx[gg as usize] {
                            break;
                        }
                        debug_assert!(
                            extensions[frame_repeat_idx[gg as usize] as usize].frame == gg
                        );
                        if extensions[frame_repeat_idx[gg as usize] as usize].id
                            != extensions[i as usize].id
                        {
                            break;
                        }
                        if extensions[frame_repeat_idx[gg as usize] as usize].id < 32
                            && extensions[frame_repeat_idx[gg as usize] as usize].len
                                != extensions[i as usize].len
                        {
                            break;
                        }
                        gg += 1;
                    }
                    g = gg;
                    if g < nb_frames {
                        break;
                    }
                    /* We can! */
                    /* If this is a long extension, save the index of the last
                       instance, so we can modify its L flag. */
                    if extensions[i as usize].id >= 32 {
                        last_long_idx = frame_repeat_idx[(nb_frames - 1) as usize];
                    }
                    /* Advance the repeat pointers. */
                    let mut g2 = f + 1;
                    while g2 < nb_frames {
                        let mut j = frame_repeat_idx[g2 as usize] + 1;
                        while j < frame_max_idx[g2 as usize] && extensions[j as usize].frame != g2 {
                            j += 1;
                        }
                        frame_repeat_idx[g2 as usize] = j;
                        g2 += 1;
                    }
                    repeat_count += 1;
                    /* Point the repeat pointer for this frame to the current
                       extension, so we know when to trigger the repeats. */
                    frame_repeat_idx[f as usize] = i;
                }
                i += 1;
            }
        }
        i = frame_min_idx[f as usize];
        while i < frame_max_idx[f as usize] {
            if extensions[i as usize].frame == f {
                /* Insert separator when needed. */
                if f != curr_frame {
                    let diff = f - curr_frame;
                    if len - pos < 2 {
                        return OPUS_BUFFER_TOO_SMALL;
                    }
                    if diff == 1 {
                        if let Some(d) = data.as_deref_mut() {
                            d[pos as usize] = 0x02;
                        }
                        pos += 1;
                    } else {
                        if let Some(d) = data.as_deref_mut() {
                            d[pos as usize] = 0x03;
                        }
                        pos += 1;
                        if let Some(d) = data.as_deref_mut() {
                            d[pos as usize] = diff as u8;
                        }
                        pos += 1;
                    }
                    curr_frame = f;
                }

                pos = write_extension(
                    &mut data,
                    len,
                    pos,
                    &extensions[i as usize],
                    if written == nb_extensions - 1 { 1 } else { 0 },
                );
                if pos < 0 {
                    return pos;
                }
                written += 1;

                if repeat_count > 0 && frame_repeat_idx[f as usize] == i {
                    let nb_repeated;
                    let last;
                    /* Add the repeat indicator. */
                    nb_repeated = repeat_count * (nb_frames - (f + 1));
                    last = (written + nb_repeated == nb_extensions)
                        || (last_long_idx < 0 && i + 1 >= frame_max_idx[f as usize]);
                    if len - pos < 1 {
                        return OPUS_BUFFER_TOO_SMALL;
                    }
                    if let Some(d) = data.as_deref_mut() {
                        d[pos as usize] = (0x04 + if last { 0 } else { 1 }) as u8;
                    }
                    pos += 1;
                    let mut g = f + 1;
                    while g < nb_frames {
                        let mut j = frame_min_idx[g as usize];
                        while j < frame_repeat_idx[g as usize] {
                            if extensions[j as usize].frame == g {
                                pos = write_extension_payload(
                                    &mut data,
                                    len,
                                    pos,
                                    &extensions[j as usize],
                                    if last && j == last_long_idx { 1 } else { 0 },
                                );
                                if pos < 0 {
                                    return pos;
                                }
                                written += 1;
                            }
                            j += 1;
                        }
                        frame_min_idx[g as usize] = j;
                        g += 1;
                    }
                    if last {
                        curr_frame += 1;
                    }
                }
            }
            i += 1;
        }
        f += 1;
    }
    debug_assert!(written == nb_extensions);
    /* If we need to pad, just prepend 0x01 bytes. Even better would be to fill
       the end with zeros, but that requires checking that turning the last
       extension into an L=1 case still fits. */
    if pad != 0 && pos < len {
        let padding = len - pos;
        if let Some(d) = data.as_deref_mut() {
            if (pos + padding) as usize <= d.len() {
                d.copy_within(0..pos as usize, padding as usize);
                let mut j = 0;
                while j < padding {
                    d[j as usize] = 0x01;
                    j += 1;
                }
            }
        }
        pos += padding;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_per_frame_table() {
        /* TOC config -> 48 kHz frame size, as the C switch produces it. */
        for config in 0..32u8 {
            let toc = config << 3;
            let fs = opus_packet_get_samples_per_frame(&[toc], 48000);
            let expected = if toc & 0x80 != 0 {
                (48000i32 << ((toc >> 3) & 0x3)) / 400
            } else if toc & 0x60 == 0x60 {
                if toc & 0x08 != 0 { 48000 / 50 } else { 48000 / 100 }
            } else {
                let a = (toc >> 3) & 0x3;
                if a == 3 { 48000 * 60 / 1000 } else { (48000 << a) / 100 }
            };
            assert_eq!(fs, expected, "config {config}");
        }
        /* Known values: 2.5, 5, 10 and 20 ms with the CELT-only signalling. */
        assert_eq!(opus_packet_get_samples_per_frame(&[0x80], 48000), 120);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x88], 48000), 240);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x90], 48000), 480);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x98], 48000), 960);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x00], 48000), 480);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x18], 48000), 2880);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x60], 48000), 480);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x68], 48000), 960);
        /* Other sample rates. */
        assert_eq!(opus_packet_get_samples_per_frame(&[0x00], 8000), 80);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x80], 8000), 20);
        assert_eq!(opus_packet_get_samples_per_frame(&[0x80], 16000), 40);
    }

    #[test]
    fn toc_parsing() {
        /* Bandwidth. */
        assert_eq!(opus_packet_get_bandwidth(&[0x00]), OPUS_BANDWIDTH_NARROWBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0x20]), OPUS_BANDWIDTH_MEDIUMBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0x40]), OPUS_BANDWIDTH_WIDEBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0x60]), OPUS_BANDWIDTH_SUPERWIDEBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0x70]), OPUS_BANDWIDTH_FULLBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0x80]), OPUS_BANDWIDTH_NARROWBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0xA0]), OPUS_BANDWIDTH_WIDEBAND);
        assert_eq!(opus_packet_get_bandwidth(&[0xE0]), OPUS_BANDWIDTH_FULLBAND);

        /* Channels / frames / samples. */
        assert_eq!(opus_packet_get_nb_channels(&[0x00]), 1);
        assert_eq!(opus_packet_get_nb_channels(&[0x04]), 2);
        assert_eq!(opus_packet_get_nb_frames(&[0x00], 1), 1);
        assert_eq!(opus_packet_get_nb_frames(&[0x01], 1), 2);
        assert_eq!(opus_packet_get_nb_frames(&[0x02], 1), 2);
        assert_eq!(opus_packet_get_nb_frames(&[0x03], 1), OPUS_INVALID_PACKET);
        assert_eq!(opus_packet_get_nb_frames(&[0x03, 0x05], 2), 5);
        assert_eq!(opus_packet_get_nb_frames(&[0x00], 0), OPUS_BAD_ARG);
        assert_eq!(opus_packet_get_nb_samples(&[0x00], 1, 48000), 480);
        /* 120 ms is the limit. */
        assert_eq!(opus_packet_get_nb_samples(&[0x18], 1, 48000), 2880);
    }

    #[test]
    fn parse_single_and_multi_frame() {
        /* One frame, code 0: TOC 0x00, 10 ms at 48 kHz. */
        let pkt = [0x00u8, 1, 2, 3];
        let mut toc = 0u8;
        let mut offset = 0i32;
        let mut frames: [&[u8]; 48] = [&[]; 48];
        let mut size = [0i16; 48];
        let ret = opus_packet_parse(&pkt, 4, &mut toc, &mut frames, &mut size, &mut offset);
        assert_eq!(ret, 1);
        assert_eq!(toc, 0x00);
        assert_eq!(offset, 1);
        assert_eq!(size[0], 3);
        assert_eq!(frames[0], &[1u8, 2, 3]);

        /* Two CBR frames, code 1: payload split in half. */
        let pkt = [0x01u8, 1, 2, 3, 4];
        let ret = opus_packet_parse(&pkt, 5, &mut toc, &mut frames, &mut size, &mut offset);
        assert_eq!(ret, 2);
        assert_eq!(size[0], 2);
        assert_eq!(size[1], 2);
        assert_eq!(frames[0], &[1u8, 2]);
        assert_eq!(frames[1], &[3u8, 4]);

        /* Code 1 with an odd payload is invalid. */
        let pkt = [0x01u8, 1, 2, 3];
        assert_eq!(
            opus_packet_parse(&pkt, 4, &mut toc, &mut frames, &mut size, &mut offset),
            OPUS_INVALID_PACKET
        );

        /* Code 2: explicit first-frame size (1 byte). */
        let pkt = [0x02u8, 2, 1, 2, 3, 4];
        let ret = opus_packet_parse(&pkt, 6, &mut toc, &mut frames, &mut size, &mut offset);
        assert_eq!(ret, 2);
        assert_eq!(size[0], 2);
        assert_eq!(size[1], 2);
        assert_eq!(frames[1], &[3u8, 4]);
        assert_eq!(offset, 2);

        /* Code 3 VBR with three frames and no padding. */
        let pkt = [0x03u8, 0x83, 1, 1, 2, 3, 4, 5];
        let ret = opus_packet_parse(&pkt, 8, &mut toc, &mut frames, &mut size, &mut offset);
        assert_eq!(ret, 3);
        assert_eq!(size[0], 1);
        assert_eq!(size[1], 1);
        assert_eq!(size[2], 2);
        assert_eq!(frames[0], &[2u8]);
        assert_eq!(frames[2], &[4u8, 5]);

        /* Code 3 CBR with one padding byte at the end. */
        let pkt = [0x03u8, 0x42, 1, 1, 2, 3, 4, 0];
        let ret = opus_packet_parse(&pkt, 8, &mut toc, &mut frames, &mut size, &mut offset);
        assert_eq!(ret, 2);
        assert_eq!(size[0], 2);
        assert_eq!(size[1], 2);
        assert_eq!(frames[0], &[1u8, 2]);
        assert_eq!(frames[1], &[3u8, 4]);

        /* Empty packet. */
        assert_eq!(
            opus_packet_parse(&[], 0, &mut toc, &mut frames, &mut size, &mut offset),
            OPUS_INVALID_PACKET
        );
        assert_eq!(
            opus_packet_parse(&[], -1, &mut toc, &mut frames, &mut size, &mut offset),
            OPUS_BAD_ARG
        );
    }

    #[test]
    fn parse_padding() {
        let pkt = [0x03u8, 0x42, 1, 1, 2, 3, 4, 0];
        let p = opus_packet_parse_impl(&pkt, 8, false).unwrap();
        assert_eq!(p.count, 2);
        assert_eq!(p.padding, &[0u8]);
        assert_eq!(p.packet_offset, 8);
        assert_eq!(p.payload_offset, 3);
    }

    #[test]
    fn parse_self_delimited() {
        /* [toc][size][payload] */
        let pkt = [0x00u8, 3, 1, 2, 3];
        let p = opus_packet_parse_impl(&pkt, 5, true).unwrap();
        assert_eq!(p.count, 1);
        assert_eq!(p.size[0], 3);
        assert_eq!(p.frames[0], &[1u8, 2, 3]);
    }

    #[test]
    fn extension_round_trip() {
        let exts = vec![
            OpusExtensionData { id: 3, frame: 0, data: vec![0xAB], len: 1 },
            OpusExtensionData { id: 0x40, frame: 1, data: vec![1, 2, 3], len: 3 },
        ];
        let n = opus_packet_extensions_generate(None, 1000, &exts, 2, 3, 0);
        assert!(n > 0);
        let mut buf = vec![0u8; n as usize];
        let n2 = opus_packet_extensions_generate(Some(&mut buf), n, &exts, 2, 3, 0);
        assert_eq!(n, n2);
        assert_eq!(opus_packet_extensions_count(&buf, n, 3), 2);
        let mut parsed = vec![OpusExtensionData::default(); 4];
        let mut cap = 4i32;
        let ret = opus_packet_extensions_parse(&buf, n, &mut parsed, &mut cap, 3);
        assert_eq!(ret, 0);
        assert_eq!(cap, 2);
        assert_eq!(parsed[0].id, 3);
        assert_eq!(parsed[0].frame, 0);
        assert_eq!(parsed[0].data, vec![0xAB]);
        assert_eq!(parsed[1].id, 0x40);
        assert_eq!(parsed[1].frame, 1);
        assert_eq!(parsed[1].data, vec![1, 2, 3]);
    }
}

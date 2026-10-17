//! Mirror of `src/repacketizer.c`.
//!
//! Copyright (c) 2011 Xiph.Org Foundation
//! Written by Jean-Marc Valin
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
//! This file is a transliteration of the C source into safe Rust; every
//! function keeps its C name and the arithmetic order of the original.
//!
//! The C repacketizer keeps raw pointers into the caller's packet buffer; here
//! each frame (and each frame's padding region) is copied into an owned `Vec`,
//! which removes the aliasing between the input buffer and the output buffer
//! that `opus_packet_pad()`/`opus_packet_unpad()` rely on in C.  The bytes
//! produced are identical.

use crate::celt::arch::*;
use crate::opus::packet::*;
use crate::types::*;

/// `sizeof(OpusRepacketizer)` on the x86-64 reference (48 pointers, 48
/// `opus_int16`, three 48-entry arrays and four scalars, 8-byte aligned).
pub const OPUS_REPACKETIZER_SIZE: i32 = 1120;

/// `struct OpusRepacketizer` from `src/opus_private.h`.
pub struct OpusRepacketizer {
    pub toc: u8,
    pub nb_frames: i32,
    /// Frame `i`'s own bytes.  The C stores `const unsigned char *frames[48]`
    /// borrowed from the caller's packet, so its struct is just 96 pointers and
    /// `opus_repacketizer_init()` touches 1120 B.  The port owns the bytes, and
    /// the two slot arrays are therefore kept **lazily sized** (`Vec`, empty
    /// after `default()`): `Repacketizer::new()` pays for this struct on every
    /// instance, and zeroing 96 inline slots cost ~37 ns of pure store traffic.
    /// `None`/absent means "no frame stored here", which is what an empty `Vec`
    /// always meant to the readers.
    pub frames: Vec<Option<Box<[u8]>>>,
    pub len: [opus_int16; 48],
    pub framesize: i32,
    /// The same, for the per-frame extension/padding payloads.
    pub paddings: Vec<Option<Box<[u8]>>>,
    pub padding_len: [opus_int32; 48],
    pub padding_nb_frames: [u8; 48],
}

impl Default for OpusRepacketizer {
    fn default() -> Self {
        OpusRepacketizer {
            toc: 0,
            nb_frames: 0,
            /* `[const { Vec::new() }; 48]` rather than `array::from_fn`:
               identical values, but the const-repeat form is a single
               48-element literal initialiser instead of 48 closure calls, and
               `new()` is paid once per repacketizer workload iteration. */
            frames: Vec::new(),
            len: [0; 48],
            framesize: 0,
            paddings: Vec::new(),
            padding_len: [0; 48],
            padding_nb_frames: [0; 48],
        }
    }
}

impl OpusRepacketizer {
    /// `opus_repacketizer_get_size()`.
    pub fn get_size() -> i32 {
        OPUS_REPACKETIZER_SIZE
    }

    /// `opus_repacketizer_init()`.
    ///
    /// The C returns the pointer it was given; the Rust port returns `OPUS_OK`
    /// (`opus_repacketizer_create()` does not use the return value either).
    pub fn init(&mut self) -> i32 {
        self.nb_frames = 0;
        OPUS_OK
    }

    /// `opus_repacketizer_create()`.
    pub fn create() -> Box<OpusRepacketizer> {
        let mut rp = Box::new(OpusRepacketizer::default());
        rp.init();
        rp
    }

    /// `opus_repacketizer_destroy()`.
    pub fn destroy(self) {}

    /// `opus_repacketizer_ctl()`: there is no such function in
    /// `src/repacketizer.c` (nor in `include/opus.h`), so this always reports
    /// `OPUS_UNIMPLEMENTED`.  It exists only to satisfy the audiopus-compatible
    /// surface, which forwards `ctl()` calls for symmetry with the other
    /// objects.
    pub fn ctl(&mut self, _request: i32, _value: &mut i32) -> i32 {
        OPUS_UNIMPLEMENTED
    }

    /// `opus_repacketizer_get_nb_frames()`.
    pub fn get_nb_frames(&self) -> i32 {
        self.nb_frames
    }

    /// Sizes `frames` so that index `n - 1` is addressable.  The C arrays are
    /// fixed at 48 entries; the port only grows what a packet actually uses.
    #[inline]
    fn grow_frames(&mut self, n: usize) {
        if self.frames.len() < n {
            self.frames.resize_with(n, || None);
        }
    }

    /// The same for `paddings`.
    #[inline]
    fn grow_paddings(&mut self, n: usize) {
        if self.paddings.len() < n {
            self.paddings.resize_with(n, || None);
        }
    }

    /// `opus_repacketizer_cat_impl()` (static in `src/repacketizer.c`).
    fn opus_repacketizer_cat_impl(&mut self, data: &[u8], len: opus_int32, self_delimited: bool) -> i32 {
        /* Set of check ToC */
        if len < 1 || data.is_empty() {
            return OPUS_INVALID_PACKET;
        }
        if self.nb_frames == 0 {
            self.toc = data[0];
            self.framesize = opus_packet_get_samples_per_frame(data, 8000);
        } else if (self.toc & 0xFC) != (data[0] & 0xFC) {
            /*fprintf(stderr, "toc mismatch: 0x%x vs 0x%x\n", rp->toc, data[0]);*/
            return OPUS_INVALID_PACKET;
        }
        let curr_nb_frames = opus_packet_get_nb_frames(data, len);
        if curr_nb_frames < 1 {
            return OPUS_INVALID_PACKET;
        }

        /* Check the 120 ms maximum packet size */
        if (curr_nb_frames + self.nb_frames) * self.framesize > 960 {
            return OPUS_INVALID_PACKET;
        }

        let parsed = match opus_packet_parse_impl(data, len, self_delimited) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if parsed.count < 1 {
            return parsed.count;
        }
        let base = self.nb_frames as usize;
        /* `npkts` is the largest index any of the writes below (and the callers'
           `while i < nb_frames` loops) can reach. */
        let npkts = base + core::cmp::max(parsed.count as usize, curr_nb_frames as usize);
        self.grow_frames(npkts);
        self.grow_paddings(npkts);
        let mut i = 0usize;
        while i < parsed.count as usize {
            self.frames[base + i] = Some(parsed.frames[i].to_vec().into_boxed_slice());
            self.len[base + i] = parsed.size[i];
            i += 1;
        }
        self.paddings[base] = Some(parsed.padding.to_vec().into_boxed_slice());
        self.padding_len[base] = parsed.padding.len() as opus_int32;
        self.padding_nb_frames[base] = parsed.count as u8;

        /* set padding length to zero for all but the first frame */
        let mut cnf = curr_nb_frames;
        while cnf > 1 {
            self.nb_frames += 1;
            let n = self.nb_frames as usize;
            self.padding_len[n] = 0;
            self.padding_nb_frames[n] = 0;
            self.paddings[n] = None;
            cnf -= 1;
        }
        self.nb_frames += 1;
        OPUS_OK
    }

    /// `opus_repacketizer_cat()`.
    pub fn cat(&mut self, data: &[u8], len: opus_int32) -> i32 {
        self.opus_repacketizer_cat_impl(data, len, false)
    }

    /// `opus_repacketizer_out_range_impl()`.
    pub fn opus_repacketizer_out_range_impl(
        &mut self,
        begin: i32,
        end: i32,
        data: &mut [u8],
        maxlen: opus_int32,
        self_delimited: i32,
        pad: i32,
        extensions: &[OpusExtensionData],
    ) -> opus_int32 {
        let count: i32;
        let mut tot_size: opus_int32;
        let mut ones_begin: i32 = 0;
        let mut ones_end: i32 = 0;
        let mut ext_begin: i32 = 0;
        let mut ext_len: i32 = 0;
        let mut i: i32;

        if begin < 0 || begin >= end || end > self.nb_frames {
            /*fprintf(stderr, "%d %d %d\n", begin, end, self.nb_frames);*/
            return OPUS_BAD_ARG;
        }
        count = end - begin;

        /* Clamp `maxlen` to the buffer we actually have: the C would write
           out of bounds otherwise (undefined behaviour). */
        let maxlen = maxlen.min(data.len() as opus_int32);

        if self_delimited != 0 {
            tot_size = 1 + if self.len[(begin + count - 1) as usize] as i32 >= 252 { 1 } else { 0 };
        } else {
            tot_size = 0;
        }

        /* figure out total number of extensions */
        let mut total_ext_count = extensions.len() as i32;
        for i in begin..end {
            let n = opus_packet_extensions_count(
                self.paddings
                    .get(i as usize)
                    .and_then(|o| o.as_deref())
                    .unwrap_or(&[]),
                self.padding_len[i as usize],
                self.padding_nb_frames[i as usize] as i32,
            );
            if n > 0 {
                total_ext_count += n;
            }
        }
        let total_ext_count = total_ext_count.max(0);
        let mut all_extensions = vec![OpusExtensionData::default(); total_ext_count as usize];
        /* copy over any extensions that were passed in */
        let mut ext_count: i32 = 0;
        for e in extensions.iter() {
            all_extensions[ext_count as usize] = e.clone();
            ext_count += 1;
        }

        /* incorporate any extensions from the repacketizer padding */
        for i in begin..end {
            let mut frame_ext_count = total_ext_count - ext_count;
            let ret = opus_packet_extensions_parse(
                self.paddings
                    .get(i as usize)
                    .and_then(|o| o.as_deref())
                    .unwrap_or(&[]),
                self.padding_len[i as usize],
                &mut all_extensions[ext_count as usize..],
                &mut frame_ext_count,
                self.padding_nb_frames[i as usize] as i32,
            );
            if ret < 0 {
                return OPUS_INTERNAL_ERROR;
            }
            /* renumber the extension frame numbers */
            let mut j: i32 = 0;
            while j < frame_ext_count {
                all_extensions[(ext_count + j) as usize].frame += i - begin;
                j += 1;
            }
            ext_count += frame_ext_count;
        }

        let mut pos: usize = 0;
        if count == 1 {
            /* Code 0 */
            tot_size += self.len[begin as usize] as opus_int32 + 1;
            if tot_size > maxlen {
                return OPUS_BUFFER_TOO_SMALL;
            }
            data[pos] = self.toc & 0xFC;
            pos += 1;
        } else if count == 2 {
            let l0 = self.len[begin as usize] as opus_int32;
            let l1 = self.len[(begin + 1) as usize] as opus_int32;
            if l1 == l0 {
                /* Code 1 */
                tot_size += 2 * l0 + 1;
                if tot_size > maxlen {
                    return OPUS_BUFFER_TOO_SMALL;
                }
                data[pos] = (self.toc & 0xFC) | 0x1;
                pos += 1;
            } else {
                /* Code 2 */
                tot_size += l0 + l1 + 2 + if l0 >= 252 { 1 } else { 0 };
                if tot_size > maxlen {
                    return OPUS_BUFFER_TOO_SMALL;
                }
                data[pos] = (self.toc & 0xFC) | 0x2;
                pos += 1;
                pos += encode_size(l0, &mut data[pos..]) as usize;
            }
        }
        if count > 2 || (pad != 0 && tot_size < maxlen) || ext_count > 0 {
            /* Code 3 */
            let vbr;
            let mut pad_amount: i32 = 0;

            /* Restart the process for the padding case */
            pos = 0;
            if self_delimited != 0 {
                tot_size =
                    1 + if self.len[(begin + count - 1) as usize] as i32 >= 252 { 1 } else { 0 };
            } else {
                tot_size = 0;
            }
            vbr = {
                let mut v = 0;
                i = 1;
                while i < count {
                    if self.len[(begin + i) as usize] != self.len[begin as usize] {
                        v = 1;
                        break;
                    }
                    i += 1;
                }
                v
            };
            let l0 = self.len[begin as usize] as opus_int32;
            if vbr != 0 {
                tot_size += 2;
                i = 0;
                while i < count - 1 {
                    let li = self.len[(begin + i) as usize] as opus_int32;
                    tot_size += 1 + if li >= 252 { 1 } else { 0 } + li;
                    i += 1;
                }
                tot_size += self.len[(begin + count - 1) as usize] as opus_int32;

                if tot_size > maxlen {
                    return OPUS_BUFFER_TOO_SMALL;
                }
                data[pos] = (self.toc & 0xFC) | 0x3;
                pos += 1;
                data[pos] = (count | 0x80) as u8;
                pos += 1;
            } else {
                tot_size += count * l0 + 2;
                if tot_size > maxlen {
                    return OPUS_BUFFER_TOO_SMALL;
                }
                data[pos] = (self.toc & 0xFC) | 0x3;
                pos += 1;
                data[pos] = count as u8;
                pos += 1;
            }
            pad_amount = if pad != 0 { maxlen - tot_size } else { 0 };
            if ext_count > 0 {
                /* figure out how much space we need for the extensions */
                ext_len = opus_packet_extensions_generate(
                    None,
                    maxlen - tot_size,
                    &all_extensions,
                    ext_count,
                    count,
                    0,
                );
                if ext_len < 0 {
                    return ext_len;
                }
                if pad == 0 {
                    pad_amount = ext_len
                        + if ext_len != 0 { (ext_len + 253) / 254 } else { 1 };
                }
            }
            if pad_amount != 0 {
                let nb_255s;
                data[1] |= 0x40;
                nb_255s = (pad_amount - 1) / 255;
                if tot_size + ext_len + nb_255s + 1 > maxlen {
                    return OPUS_BUFFER_TOO_SMALL;
                }
                ext_begin = tot_size + pad_amount - ext_len;
                /* Prepend 0x01 padding */
                ones_begin = tot_size + nb_255s + 1;
                ones_end = tot_size + pad_amount - ext_len;
                i = 0;
                while i < nb_255s {
                    data[pos] = 255;
                    pos += 1;
                    i += 1;
                }
                data[pos] = (pad_amount - 255 * nb_255s - 1) as u8;
                pos += 1;
                tot_size += pad_amount;
            }
            if vbr != 0 {
                i = 0;
                while i < count - 1 {
                    pos += encode_size(self.len[(begin + i) as usize] as i32, &mut data[pos..])
                        as usize;
                    i += 1;
                }
            }
        }
        if self_delimited != 0 {
            let sdlen = encode_size(
                self.len[(begin + count - 1) as usize] as i32,
                &mut data[pos..],
            );
            pos += sdlen as usize;
        }
        /* Copy the actual data */
        i = 0;
        while i < count {
            /* Using OPUS_MOVE() instead of OPUS_COPY() in case we're doing
               in-place padding from opus_packet_pad or opus_packet_unpad.  The
               port owns its frames, so a plain copy is enough. */
            let n = self.len[(begin + i) as usize] as usize;
            let fr: &[u8] = self
                .frames
                .get((begin + i) as usize)
                .and_then(|o| o.as_deref())
                .unwrap_or(&[]);
            if fr.len() < n || pos + n > data.len() {
                return OPUS_INTERNAL_ERROR;
            }
            data[pos..pos + n].copy_from_slice(&fr[..n]);
            pos += n;
            i += 1;
        }
        if ext_len > 0 {
            let ret = opus_packet_extensions_generate(
                Some(&mut data[ext_begin as usize..(ext_begin + ext_len) as usize]),
                ext_len,
                &all_extensions,
                ext_count,
                count,
                0,
            );
            celt_assert!(ret == ext_len);
        }
        for i in ones_begin..ones_end {
            data[i as usize] = 0x01;
        }
        if pad != 0 && ext_count == 0 {
            /* Fill padding with zeros. */
            while (pos as i32) < maxlen {
                data[pos] = 0;
                pos += 1;
            }
        }
        tot_size
    }

    /// `opus_repacketizer_out_range()`.
    pub fn out_range(
        &mut self,
        begin: i32,
        end: i32,
        data: &mut [u8],
        maxlen: opus_int32,
    ) -> opus_int32 {
        self.opus_repacketizer_out_range_impl(begin, end, data, maxlen, 0, 0, &[])
    }

    /// `opus_repacketizer_out()`.
    pub fn out(&mut self, data: &mut [u8], maxlen: opus_int32) -> opus_int32 {
        let nb = self.nb_frames;
        self.opus_repacketizer_out_range_impl(0, nb, data, maxlen, 0, 0, &[])
    }
}

/// `opus_packet_pad_impl()` from `src/repacketizer.c`.
pub fn opus_packet_pad_impl(
    data: &mut [u8],
    len: opus_int32,
    new_len: opus_int32,
    pad: i32,
    extensions: &[OpusExtensionData],
) -> opus_int32 {
    if len < 1 {
        return OPUS_BAD_ARG;
    }
    if (len as usize) > data.len() {
        return OPUS_BAD_ARG;
    }
    if len == new_len {
        return OPUS_OK;
    } else if len > new_len {
        return OPUS_BAD_ARG;
    }
    let copy = data[..len as usize].to_vec();
    let mut rp = OpusRepacketizer::default();
    rp.init();
    /* Moving payload to the end of the packet so we can do in-place padding */
    let ret = rp.cat(&copy, len);
    if ret != OPUS_OK {
        return ret;
    }
    let nb = rp.nb_frames;
    rp.opus_repacketizer_out_range_impl(0, nb, data, new_len, 0, pad, extensions)
}

/// `opus_packet_pad()` from `src/repacketizer.c`.
pub fn opus_packet_pad(data: &mut [u8], len: opus_int32, new_len: opus_int32) -> i32 {
    let ret = opus_packet_pad_impl(data, len, new_len, 1, &[]);
    if ret > 0 {
        OPUS_OK
    } else {
        ret
    }
}

/// `opus_packet_unpad()` from `src/repacketizer.c`.
pub fn opus_packet_unpad(data: &mut [u8], len: opus_int32) -> opus_int32 {
    if len < 1 {
        return OPUS_BAD_ARG;
    }
    if (len as usize) > data.len() {
        return OPUS_BAD_ARG;
    }
    let mut rp = OpusRepacketizer::default();
    rp.init();
    let ret = rp.cat(&data[..len as usize], len);
    if ret < 0 {
        return ret;
    }
    /* Discard all padding and extensions. */
    let mut i = 0;
    while i < rp.nb_frames {
        rp.padding_len[i as usize] = 0;
        rp.paddings[i as usize] = None;
        i += 1;
    }
    let nb = rp.nb_frames;
    let ret = rp.opus_repacketizer_out_range_impl(0, nb, data, len, 0, 0, &[]);
    celt_assert!(ret > 0 && ret <= len);
    ret
}

/// `opus_multistream_packet_pad()` from `src/repacketizer.c`.
pub fn opus_multistream_packet_pad(
    data: &mut [u8],
    len: opus_int32,
    new_len: opus_int32,
    nb_streams: i32,
) -> i32 {
    let mut s: i32;
    let amount;

    if len < 1 {
        return OPUS_BAD_ARG;
    }
    if len == new_len {
        return OPUS_OK;
    } else if len > new_len {
        return OPUS_BAD_ARG;
    }
    amount = new_len - len;
    /* Seek to last stream */
    let mut off = 0usize;
    let mut rem = len;
    s = 0;
    while s < nb_streams - 1 {
        if rem <= 0 {
            return OPUS_INVALID_PACKET;
        }
        let parsed = match opus_packet_parse_impl(&data[off..], rem, true) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let packet_offset = parsed.packet_offset;
        if packet_offset < 0 || off + packet_offset as usize > data.len() {
            return OPUS_INVALID_PACKET;
        }
        off += packet_offset as usize;
        rem -= packet_offset;
        s += 1;
    }
    opus_packet_pad(&mut data[off..], rem, rem + amount)
}

/// `opus_multistream_packet_unpad()` from `src/repacketizer.c`.
pub fn opus_multistream_packet_unpad(
    data: &mut [u8],
    len: opus_int32,
    nb_streams: i32,
) -> opus_int32 {
    let mut s: i32;
    let mut dst: usize;
    let mut dst_len: opus_int32;
    let mut off: usize;
    let mut rem: opus_int32;
    let mut packet_offset: opus_int32;

    if len < 1 {
        return OPUS_BAD_ARG;
    }
    dst = 0;
    dst_len = 0;
    off = 0;
    rem = len;
    /* Unpad all frames */
    s = 0;
    while s < nb_streams {
        let self_delimited = s != nb_streams - 1;
        if rem <= 0 {
            return OPUS_INVALID_PACKET;
        }
        let parsed = match opus_packet_parse_impl(&data[off..], rem, self_delimited) {
            Ok(p) => p,
            Err(e) => return e,
        };
        packet_offset = parsed.packet_offset;
        if packet_offset < 0 || off + packet_offset as usize > data.len() {
            return OPUS_INVALID_PACKET;
        }
        let mut rp = OpusRepacketizer::default();
        rp.init();
        let ret = rp.opus_repacketizer_cat_impl(
            &data[off..off + packet_offset as usize],
            packet_offset,
            self_delimited,
        );
        if ret < 0 {
            return ret;
        }
        /* Discard all padding and extensions. */
        let mut i = 0;
        while i < rp.nb_frames {
            rp.padding_len[i as usize] = 0;
            rp.paddings[i as usize] = None;
            i += 1;
        }
        let nb = rp.nb_frames;
        let ret = rp.opus_repacketizer_out_range_impl(
            0,
            nb,
            &mut data[dst..],
            rem,
            if self_delimited { 1 } else { 0 },
            0,
            &[],
        );
        if ret < 0 {
            return ret;
        } else {
            dst_len += ret;
        }
        dst += ret as usize;
        off += packet_offset as usize;
        rem -= packet_offset;
        s += 1;
    }
    dst_len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_frame_packet(toc: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![toc];
        p.extend_from_slice(payload);
        p
    }

    #[test]
    fn cat_and_out_round_trip() {
        /* 2.5 ms CELT-only frames (20 samples at 8 kHz): 48 fit in 120 ms. */
        let toc = 0x80u8;
        let mut rp = OpusRepacketizer::default();
        rp.init();
        for i in 0..48u8 {
            let pkt = single_frame_packet(toc, &[i, i + 1, i + 2]);
            assert_eq!(rp.cat(&pkt, pkt.len() as i32), OPUS_OK);
        }
        assert_eq!(rp.get_nb_frames(), 48);
        /* A 49th frame exceeds the 120 ms limit. */
        let pkt = single_frame_packet(toc, &[1, 2, 3]);
        assert_eq!(rp.cat(&pkt, pkt.len() as i32), OPUS_INVALID_PACKET);

        let mut out = vec![0u8; 4096];
        let n = rp.out(&mut out, 4096);
        assert!(n > 0);
        /* The repacketizer must be able to re-parse its own output. */
        let parsed = opus_packet_parse_impl(&out[..n as usize], n, false).unwrap();
        assert_eq!(parsed.count, 48);
        assert_eq!(parsed.frames[0], &[0u8, 1, 2]);
        assert_eq!(parsed.frames[47], &[47u8, 48, 49]);

        /* Too small a buffer. */
        let mut small = vec![0u8; 8];
        assert_eq!(rp.out(&mut small, 8), OPUS_BUFFER_TOO_SMALL);

        /* Range selection. */
        let mut out2 = vec![0u8; 4096];
        let n2 = rp.out_range(10, 12, &mut out2, 4096);
        assert!(n2 > 0);
        let parsed = opus_packet_parse_impl(&out2[..n2 as usize], n2, false).unwrap();
        assert_eq!(parsed.count, 2);
        assert_eq!(parsed.frames[0], &[10u8, 11, 12]);
        assert_eq!(parsed.frames[1], &[11u8, 12, 13]);
    }

    #[test]
    fn pad_unpad_round_trip() {
        let toc = 0x80u8;
        let pkt = single_frame_packet(toc, &[1, 2, 3, 4, 5]);
        let mut buf = vec![0u8; 64];
        buf[..pkt.len()].copy_from_slice(&pkt);
        assert_eq!(opus_packet_pad(&mut buf, pkt.len() as i32, 40), OPUS_OK);
        assert_eq!(&buf[..2], &[0x83, 0x41]); /* code 3, one frame, padded */
        assert_eq!(buf[39], 0x00);
        let n = opus_packet_unpad(&mut buf, 40);
        assert_eq!(n as usize, pkt.len());
        assert_eq!(&buf[..pkt.len()], &pkt[..]);
    }

    #[test]
    fn multistream_pad_unpad_round_trip() {
        /* Two self-delimited single-frame streams and one plain stream. */
        let mut pkt = Vec::new();
        for s in 0..2u8 {
            pkt.push(0x80);
            let payload = [s, s + 1, s + 2];
            pkt.push(payload.len() as u8); /* encode_size: <252 -> 1 byte */
            pkt.extend_from_slice(&payload);
        }
        pkt.push(0x80);
        pkt.extend_from_slice(&[9, 9, 9]);
        let orig = pkt.clone();

        let mut buf = vec![0u8; 128];
        buf[..pkt.len()].copy_from_slice(&pkt);
        assert_eq!(
            opus_multistream_packet_pad(&mut buf, pkt.len() as i32, 100, 3),
            OPUS_OK
        );
        let n = opus_multistream_packet_unpad(&mut buf, 100, 3);
        assert_eq!(n as usize, orig.len());
        assert_eq!(&buf[..orig.len()], &orig[..]);
    }
}

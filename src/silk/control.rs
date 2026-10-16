//! Mirror of `silk/control.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Encoder/decoder control structures.  `FLAG_DECODE_*` are the decoder API flags.  The `#ifdef ENABLE_OSCE_BWE` fields of `silk_DecControlStruct` are dropped: `ENABLE_OSCE_BWE` is not defined in the reference configuration.
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
use crate::types::*;

/* Decoder API flags */
/// `FLAG_DECODE_NORMAL`
pub const FLAG_DECODE_NORMAL: opus_int = 0;
/// `FLAG_PACKET_LOST`
pub const FLAG_PACKET_LOST: opus_int = 1;
/// `FLAG_DECODE_LBRR`
pub const FLAG_DECODE_LBRR: opus_int = 2;

/// Structure for controlling encoder operation (`silk_EncControlStruct`).
#[derive(Clone, Copy, Debug, Default)]
pub struct silk_EncControlStruct {
    /// I: Number of channels; 1/2
    pub nChannelsAPI: opus_int32,
    /// I: Number of channels; 1/2
    pub nChannelsInternal: opus_int32,
    /// I: Input signal sampling rate in Hertz; 8000/12000/16000/24000/32000/44100/48000
    pub API_sampleRate: opus_int32,
    /// I: Maximum internal sampling rate in Hertz; 8000/12000/16000
    pub maxInternalSampleRate: opus_int32,
    /// I: Minimum internal sampling rate in Hertz; 8000/12000/16000
    pub minInternalSampleRate: opus_int32,
    /// I: Soft request for internal sampling rate in Hertz; 8000/12000/16000
    pub desiredInternalSampleRate: opus_int32,
    /// I: Number of samples per packet in milliseconds; 10/20/40/60
    pub payloadSize_ms: opus_int,
    /// I: Bitrate during active speech in bits/second; internally limited
    pub bitRate: opus_int32,
    /// I: Uplink packet loss in percent (0-100)
    pub packetLossPercentage: opus_int,
    /// I: Complexity mode; 0 is lowest, 10 is highest complexity
    pub complexity: opus_int,
    /// I: Flag to enable in-band Forward Error Correction (FEC); 0/1
    pub useInBandFEC: opus_int,
    /// I: Flag to enable in-band Deep REDundancy (DRED); 0/1
    pub useDRED: opus_int,
    /// I: Flag to actually code in-band FEC in the current packet; 0/1
    pub LBRR_coded: opus_int,
    /// I: Flag to enable discontinuous transmission (DTX); 0/1
    pub useDTX: opus_int,
    /// I: Flag to use constant bitrate
    pub useCBR: opus_int,
    /// I: Maximum number of bits allowed for the frame
    pub maxBits: opus_int,
    /// I: Causes a smooth downmix to mono
    pub toMono: opus_int,
    /// I: Opus encoder is allowing us to switch bandwidth
    pub opusCanSwitch: opus_int,
    /// I: Make frames as independent as possible (but still use LPC)
    pub reducedDependency: opus_int,
    /// O: Internal sampling rate used, in Hertz; 8000/12000/16000
    pub internalSampleRate: opus_int32,
    /// O: Flag that bandwidth switching is allowed (because low voice activity)
    pub allowBandwidthSwitch: opus_int,
    /// O: Flag that SILK runs in WB mode without variable LP filter
    pub inWBmodeWithoutVariableLP: opus_int,
    /// O: Stereo width
    pub stereoWidth_Q14: opus_int,
    /// O: Tells the Opus encoder we're ready to switch
    pub switchReady: opus_int,
    /// O: SILK Signal type
    pub signalType: opus_int,
    /// O: SILK offset (dithering)
    pub offset: opus_int,
}

/// Structure for controlling decoder operation and reading decoder status
/// (`silk_DecControlStruct`).
#[derive(Clone, Copy, Debug, Default)]
pub struct silk_DecControlStruct {
    /// I: Number of channels; 1/2
    pub nChannelsAPI: opus_int32,
    /// I: Number of channels; 1/2
    pub nChannelsInternal: opus_int32,
    /// I: Output signal sampling rate in Hertz; 8000/12000/16000/24000/32000/44100/48000
    pub API_sampleRate: opus_int32,
    /// I: Internal sampling rate used, in Hertz; 8000/12000/16000
    pub internalSampleRate: opus_int32,
    /// I: Number of samples per packet in milliseconds; 10/20/40/60
    pub payloadSize_ms: opus_int,
    /// O: Pitch lag of previous frame (0 if unvoiced), in samples at 48 kHz
    pub prevPitchLag: opus_int,
    /// I: Enable Deep PLC
    pub enable_deep_plc: opus_int,
    /// I: OSCE method (`#ifdef ENABLE_OSCE`)
    pub osce_method: opus_int,
    /* `#ifdef ENABLE_OSCE_BWE`: enable_osce_bwe, osce_extended_mode and
       prev_osce_extended_mode are dropped — the reference build does not
       define ENABLE_OSCE_BWE. */
}
